use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use crate::{HEADER_BYTES, MAXIMUM_MESSAGE_SIZE};

use basics::base_uint::Uint256;
use protocol::{HashPrefix, PublicKey, STValidation, sha512_half, verify_digest};

use crate::message::{
    TmEndpoints, TmGetLedger, TmGetObjectByHash, TmHaveTransactions, TmLedgerData, TmManifests,
    TmProofPathRequest, TmProofPathResponse, TmProposeSet, TmReplayDeltaRequest,
    TmReplayDeltaResponse, TmTransaction, TmValidation, TmValidatorList, TmValidatorListCollection,
};
use crate::peer::{Peer, PeerId, ProtocolFeature};
use crate::peer_imp::PeerImp;

/// Maximum entries retained in the pre-router fallback queue per message
/// family. Rippled dispatches directly from the network thread to handlers
/// or JobQueue; this queue only accumulates during the brief window between
/// overlay construction and router installation at startup. The cap prevents
/// unbounded memory growth if a router is never installed or is cleared.
const FALLBACK_QUEUE_CAP: usize = 10_000;

/// One paused session simultaneously retains decoded protobuf containers and
/// the raw/decompression envelope from which they were accepted. Both maxima
/// are existing decoder bounds; this is a finite Rust representation bound,
/// not a new XRPL protocol value.
const DEFERRED_LEDGER_DATA_FRAME_CAPACITY: usize = MAXIMUM_MESSAGE_SIZE
    .saturating_mul(2)
    .saturating_add(HEADER_BYTES)
    .saturating_add(std::mem::size_of::<TmLedgerData>());

fn push_bounded<T>(queue: &mut Vec<T>, message: T, _family: &'static str) -> bool {
    if queue.len() >= FALLBACK_QUEUE_CAP {
        return false;
    }
    queue.push(message);
    true
}

fn extend_bounded<T>(queue: &mut Vec<T>, messages: Vec<T>, _family: &'static str) {
    let remaining = FALLBACK_QUEUE_CAP.saturating_sub(queue.len());
    queue.extend(messages.into_iter().take(remaining));
}

#[derive(Debug, Clone, PartialEq)]
pub struct PeerMessage<T> {
    pub peer_id: PeerId,
    pub message: T,
}

/// Result of the direct ledger-data transport handoff. `Deferred` means the
/// matching acquisition has not reserved its bounded mailbox yet; the
/// transport retains exactly this decoded frame and retries admission while
/// continuing to dispatch control traffic such as PING/PONG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerDataIngressDisposition {
    Delivered,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedEndpoint {
    pub endpoint: SocketAddr,
    pub hops: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueuedEndpoints {
    pub peer_id: PeerId,
    pub version: u32,
    pub malformed: usize,
    pub endpoints: Vec<QueuedEndpoint>,
    pub message: TmEndpoints,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueuedTransaction {
    pub peer_id: PeerId,
    pub id: Uint256,
    pub batch: bool,
    pub message: TmTransaction,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueuedProposal {
    pub peer_id: PeerId,
    pub suppression: Uint256,
    pub public_key: PublicKey,
    pub current_tx_hash: Uint256,
    pub previous_ledger: Uint256,
    pub message: TmProposeSet,
}

impl QueuedProposal {
    /// Verify the proposal's Ed25519/secp256k1 signature.
    ///
    /// Matches rippled's `RCLCxPeerPos::checkSign()` which calls
    /// `verify_digest` against the proposal signing hash.  The hash encodes
    /// `HashPrefix::Proposal (0x50525000) | propose_seq | close_time |
    /// prev_ledger | current_tx_hash`, matching `RCLCxPeerPos::hash_append`.
    ///
    /// Returns `false` if the signature is invalid or the key type is wrong.
    /// Cluster peers are exempt from this check at the call site in
    /// `overlay_impl.rs` (same as rippled's `checkPropose` cluster bypass).
    pub fn check_sign(&self) -> bool {
        let close_time = self.message.close_time;
        let propose_seq = self.message.propose_seq;
        // Proposal signing hash: HashPrefix::Proposal | seq | close_time |
        // prev_ledger | tx_hash. HashPrefix::Proposal is the canonical
        // XRPL `PRP\0` domain separator.
        let mut data = Vec::with_capacity(4 + 4 + 4 + 32 + 32);
        data.extend_from_slice(&HashPrefix::Proposal.as_u32().to_be_bytes());
        data.extend_from_slice(&propose_seq.to_be_bytes());
        data.extend_from_slice(&close_time.to_be_bytes());
        data.extend_from_slice(self.previous_ledger.data());
        data.extend_from_slice(self.current_tx_hash.data());
        let hash = sha512_half(&data);
        verify_digest(&self.public_key, hash, &self.message.signature, false)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueuedValidation {
    pub peer_id: PeerId,
    pub suppression: Uint256,
    pub message: TmValidation,
    /// Parsed before ingress admission so deferred signature verification keeps
    /// the original receipt-time `seen` value.
    pub validation: Option<STValidation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueuedHaveTransactions {
    pub peer_id: PeerId,
    pub hashes: Vec<Uint256>,
    pub message: TmHaveTransactions,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct OverlayInboundSnapshot {
    pub manifests: Vec<PeerMessage<TmManifests>>,
    pub endpoints: Vec<QueuedEndpoints>,
    pub transactions: Vec<QueuedTransaction>,
    pub get_ledgers: Vec<PeerMessage<TmGetLedger>>,
    pub ledger_data: Vec<PeerMessage<TmLedgerData>>,
    pub proposals: Vec<QueuedProposal>,
    pub validations: Vec<QueuedValidation>,
    pub validator_lists: Vec<PeerMessage<TmValidatorList>>,
    pub validator_list_collections: Vec<PeerMessage<TmValidatorListCollection>>,
    pub get_objects: Vec<PeerMessage<TmGetObjectByHash>>,
    pub have_transactions: Vec<QueuedHaveTransactions>,
    pub proof_path_requests: Vec<PeerMessage<TmProofPathRequest>>,
    pub proof_path_responses: Vec<PeerMessage<TmProofPathResponse>>,
    pub replay_delta_requests: Vec<PeerMessage<TmReplayDeltaRequest>>,
    pub replay_delta_responses: Vec<PeerMessage<TmReplayDeltaResponse>>,
}

pub trait OverlayInboundHandler: Send + Sync {
    fn on_manifests(&self, _peer: &Arc<PeerImp>, _message: TmManifests) {}
    fn on_endpoints(&self, _peer: &Arc<PeerImp>, _message: QueuedEndpoints) {}
    fn on_transaction(&self, _peer: &Arc<PeerImp>, _message: QueuedTransaction) {}
    fn on_get_ledger(&self, _peer: &Arc<PeerImp>, _message: TmGetLedger) {}
    fn on_ledger_data(
        &self,
        _peer: &Arc<PeerImp>,
        _message: TmLedgerData,
    ) -> LedgerDataIngressDisposition {
        LedgerDataIngressDisposition::Delivered
    }
    fn on_propose_ledger(&self, _peer: &Arc<PeerImp>, _message: QueuedProposal) {}
    fn on_validation(&self, _peer: &Arc<PeerImp>, _message: QueuedValidation) {}
    fn on_validator_list(&self, _peer: &Arc<PeerImp>, _message: TmValidatorList) {}
    fn on_validator_list_collection(
        &self,
        _peer: &Arc<PeerImp>,
        _message: TmValidatorListCollection,
    ) {
    }
    fn on_get_objects(&self, _peer: &Arc<PeerImp>, _message: TmGetObjectByHash) {}
    fn on_have_transactions(&self, _peer: &Arc<PeerImp>, _message: QueuedHaveTransactions) {}
    fn on_proof_path_request(&self, _peer: &Arc<PeerImp>, _message: TmProofPathRequest) {}
    fn on_proof_path_response(&self, _peer: &Arc<PeerImp>, _message: TmProofPathResponse) {}
    fn on_replay_delta_request(&self, _peer: &Arc<PeerImp>, _message: TmReplayDeltaRequest) {}
    fn on_replay_delta_response(&self, _peer: &Arc<PeerImp>, _message: TmReplayDeltaResponse) {}
}

#[derive()]
pub struct QueuedOverlayInboundHandler {
    inner: Mutex<OverlayInboundSnapshot>,
    /// Optional channel for immediate ledger_data delivery, bypassing the
    /// snapshot queue. reference processes TmLedgerData immediately via
    /// InboundLedgers::gotLedgerData on the network thread. This channel
    /// replicates that immediate delivery.
    ledger_data_tx: Mutex<Option<std::sync::mpsc::SyncSender<PeerMessage<TmLedgerData>>>>,
    /// Direct routing callback for TmLedgerData — routes immediately to
    /// acquisition threads without any channel hop. This is the fastest path,
    /// matching reference where gotLedgerData dispatches directly from the network thread.
    #[allow(clippy::type_complexity)]
    ledger_data_router: Mutex<
        Option<Arc<dyn Fn(PeerId, TmLedgerData) -> LedgerDataIngressDisposition + Send + Sync>>,
    >,
    /// One decoder-owned frame per paused peer. The session pauses reads for
    /// that peer until this entry reaches a non-deferred terminal disposition.
    /// The map is additionally byte-accounted at admission: every retained
    /// entry reserves the maximum of its decoded payload and the already
    /// decoder-bounded wire/decompression frame envelope.  OverlayImpl sets
    /// the aggregate limit from the configured finite peer/session limit.
    ledger_data_deferred: Mutex<BTreeMap<PeerId, DeferredLedgerData>>,
    deferred_ledger_data_bytes: Mutex<DeferredLedgerDataBytes>,
    /// Serializes fallback drain with all direct ledger-data delivery. A
    /// router installed during startup must replay older packets before any
    /// concurrent ingress can overtake them.
    ledger_data_delivery_gate: Mutex<()>,
    #[allow(clippy::type_complexity)]
    replay_delta_response_router:
        Mutex<Option<Arc<dyn Fn(PeerId, TmReplayDeltaResponse) + Send + Sync>>>,
    proof_path_request_router: Mutex<Option<Arc<dyn Fn(PeerId, TmProofPathRequest) + Send + Sync>>>,
    proof_path_response_router:
        Mutex<Option<Arc<dyn Fn(PeerId, TmProofPathResponse) + Send + Sync>>>,
    replay_delta_request_router:
        Mutex<Option<Arc<dyn Fn(PeerId, TmReplayDeltaRequest) + Send + Sync>>>,
    /// Direct routing callback for inbound transactions — dispatches
    /// immediately to a JobQueue worker on receipt, matching reference
    /// PeerImp::handleTransaction -> JobQueue::addJob(JtTransaction,
    /// "RcvCheckTx", ...). Without this, transactions only got processed on
    /// the next 1s overlay timer tick, which is too slow relative to
    /// consensus round-close timing and causes sporadic quorum misses.
    #[allow(clippy::type_complexity)]
    transaction_router: Mutex<Option<Arc<dyn Fn(PeerId, QueuedTransaction) + Send + Sync>>>,
    /// Direct routing callback for validations. Runtime wiring uses a blocking
    /// downstream handoff so consensus work is retained under queue pressure,
    /// matching rippled's JobQueue dispatch rather than dropping messages.
    #[allow(clippy::type_complexity)]
    validation_router: Mutex<Option<Arc<dyn Fn(QueuedValidation) + Send + Sync>>>,
    /// Notify channel for instant validation wake. When a validation arrives,
    /// a signal is sent so the validation processing thread wakes immediately
    /// instead of polling every 500ms. Matches reference where validations trigger
    /// checkAccept synchronously on the network thread.
    validation_notify_tx: Mutex<Option<std::sync::mpsc::SyncSender<()>>>,
    /// Notify callback to wake the consensus strand loop immediately when a
    /// proposal arrives. Removes the 50ms poll latency, matching rippled's
    /// strand-based immediate dispatch of proposals.
    proposal_notify: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    /// Direct routing callback for proposals — sends directly to the strand's
    /// proposal_tx channel instead of queuing. Matches rippled's event-driven
    /// model where proposals are dispatched immediately to the strand.
    #[allow(clippy::type_complexity)]
    proposal_router: Mutex<Option<Arc<dyn Fn(QueuedProposal) + Send + Sync>>>,
    /// Direct routing callback for GetLedger requests — dispatches directly
    /// to the JobQueue instead of queuing for the polling loop.
    #[allow(clippy::type_complexity)]
    get_ledger_router: Mutex<Option<Arc<dyn Fn(PeerId, TmGetLedger) + Send + Sync>>>,
    /// Direct routing callback for GetObjectByHash requests — dispatches
    /// directly to the JobQueue instead of queuing for the polling loop.
    #[allow(clippy::type_complexity)]
    get_objects_router: Mutex<Option<Arc<dyn Fn(PeerId, TmGetObjectByHash) + Send + Sync>>>,
}

#[derive(Debug)]
struct DeferredLedgerData {
    message: TmLedgerData,
    reserved_bytes: usize,
}

#[derive(Debug)]
struct DeferredLedgerDataBytes {
    current: usize,
    limit: usize,
}

impl Default for DeferredLedgerDataBytes {
    fn default() -> Self {
        // A handler constructed outside OverlayImpl can still retain one
        // decoder-bounded frame. Production replaces this with the finite
        // active-session derivation before peer sessions start.
        Self {
            current: 0,
            limit: DEFERRED_LEDGER_DATA_FRAME_CAPACITY,
        }
    }
}

fn deferred_ledger_data_bytes(message: &TmLedgerData) -> usize {
    let decoded = std::mem::size_of::<TmLedgerData>()
        .saturating_add(message.ledger_hash.capacity())
        .saturating_add(
            message
                .nodes
                .capacity()
                .saturating_mul(message.nodes.first().map_or(0, std::mem::size_of_val)),
        )
        .saturating_add(message.nodes.iter().fold(0usize, |total, node| {
            total
                .saturating_add(node.nodeid.as_ref().map_or(0, |id: &Vec<u8>| id.capacity()))
                .saturating_add(node.nodedata.capacity())
        }));
    // The session has not yet crossed its retry/cancel boundary, so raw and
    // decompressed ownership coexist with the decoded container graph. Charge
    // both envelopes rather than choosing the larger one.
    decoded.saturating_add(MAXIMUM_MESSAGE_SIZE.saturating_add(HEADER_BYTES))
}

impl Default for QueuedOverlayInboundHandler {
    fn default() -> Self {
        Self {
            inner: Mutex::new(OverlayInboundSnapshot::default()),
            ledger_data_tx: Mutex::new(None),
            ledger_data_router: Mutex::new(None),
            ledger_data_deferred: Mutex::new(BTreeMap::new()),
            deferred_ledger_data_bytes: Mutex::new(DeferredLedgerDataBytes::default()),
            ledger_data_delivery_gate: Mutex::new(()),
            replay_delta_response_router: Mutex::new(None),
            proof_path_request_router: Mutex::new(None),
            proof_path_response_router: Mutex::new(None),
            replay_delta_request_router: Mutex::new(None),
            transaction_router: Mutex::new(None),
            validation_router: Mutex::new(None),
            validation_notify_tx: Mutex::new(None),
            proposal_notify: Mutex::new(None),
            proposal_router: Mutex::new(None),
            get_ledger_router: Mutex::new(None),
            get_objects_router: Mutex::new(None),
        }
    }
}

impl QueuedOverlayInboundHandler {
    pub fn snapshot(&self) -> OverlayInboundSnapshot {
        self.inner.lock().expect("overlay inbound lock").clone()
    }

    pub fn take_snapshot(&self) -> OverlayInboundSnapshot {
        let mut guard = self.inner.lock().expect("overlay inbound lock");
        // Take everything EXCEPT get_objects (handled by bootstrap loop separately)
        let get_objects = std::mem::take(&mut guard.get_objects);
        let snapshot = std::mem::take(&mut *guard);
        guard.get_objects = get_objects;
        snapshot
    }

    /// Set the finite aggregate deferred-frame limit before sessions start.
    /// `active_session_limit` comes from OverlayImpl's configured peer limit;
    /// each session can retain at most one frame whose raw/decompressed bound
    /// is `MAXIMUM_MESSAGE_SIZE + HEADER_BYTES`.
    pub fn set_deferred_ledger_data_session_limit(&self, active_session_limit: usize) {
        let _delivery_gate = self
            .ledger_data_delivery_gate
            .lock()
            .expect("ledger_data_delivery_gate lock");
        let mut bytes = self
            .deferred_ledger_data_bytes
            .lock()
            .expect("deferred_ledger_data_bytes lock");
        bytes.limit = active_session_limit.saturating_mul(DEFERRED_LEDGER_DATA_FRAME_CAPACITY);
    }

    pub fn deferred_ledger_data_byte_snapshot(&self) -> (usize, usize) {
        let bytes = self
            .deferred_ledger_data_bytes
            .lock()
            .expect("deferred_ledger_data_bytes lock");
        (bytes.current, bytes.limit)
    }

    pub fn clear(&self) {
        let _delivery_gate = self
            .ledger_data_delivery_gate
            .lock()
            .expect("ledger_data_delivery_gate lock");
        *self.inner.lock().expect("overlay inbound lock") = OverlayInboundSnapshot::default();
        self.ledger_data_deferred
            .lock()
            .expect("ledger_data_deferred lock")
            .clear();
        self.deferred_ledger_data_bytes
            .lock()
            .expect("deferred_ledger_data_bytes lock")
            .current = 0;
    }

    /// Terminally discard the one transport-owned frame for a peer whose
    /// session has closed. The frame was never admitted to Worker 2, so no
    /// admission lease exists to release here.
    pub fn discard_deferred_ledger_data(&self, peer_id: PeerId) {
        let _delivery_gate = self
            .ledger_data_delivery_gate
            .lock()
            .expect("ledger_data_delivery_gate lock");
        let removed = self
            .ledger_data_deferred
            .lock()
            .expect("ledger_data_deferred lock")
            .remove(&peer_id);
        if let Some(frame) = removed {
            let mut bytes = self
                .deferred_ledger_data_bytes
                .lock()
                .expect("deferred_ledger_data_bytes lock");
            bytes.current = bytes.current.saturating_sub(frame.reserved_bytes);
        }
    }

    /// Retain exactly one decoded matching frame for the peer session which
    /// owns it. There is intentionally no process-global paused-peer cap:
    /// live peer/session admission owns the number of peers, while the wire
    /// decoder owns the per-frame `MAXIMUM_MESSAGE_SIZE` bound. The session
    /// keeps dispatching control frames while this entry retries, so later
    /// ledger-data frames must be dropped rather than retained or allowed to
    /// consume a second bounded slot.
    fn defer_ledger_data(
        &self,
        peer: &Arc<PeerImp>,
        message: TmLedgerData,
    ) -> LedgerDataIngressDisposition {
        let mut deferred = self
            .ledger_data_deferred
            .lock()
            .expect("ledger_data_deferred lock");
        if deferred.contains_key(&peer.id()) {
            tracing::debug!(
                target: "overlay",
                peer_id = %peer.id(),
                "Dropping ledger data while a bounded deferred frame is retained"
            );
            return LedgerDataIngressDisposition::Delivered;
        }
        let reserved_bytes = deferred_ledger_data_bytes(&message);
        let mut bytes = self
            .deferred_ledger_data_bytes
            .lock()
            .expect("deferred_ledger_data_bytes lock");
        if bytes.current.saturating_add(reserved_bytes) > bytes.limit {
            drop(bytes);
            drop(deferred);
            // This matching reply cannot be retained without exceeding the
            // finite configured active-session envelope. It was not admitted
            // to Worker 2, so disconnect is the terminal transport owner.
            peer.request_disconnect();
            return LedgerDataIngressDisposition::Delivered;
        }
        bytes.current += reserved_bytes;
        deferred.insert(
            peer.id(),
            DeferredLedgerData {
                message,
                reserved_bytes,
            },
        );
        LedgerDataIngressDisposition::Deferred
    }

    /// Register a channel for immediate TmLedgerData delivery.
    /// When set, TmLedgerData messages are sent to this channel immediately
    /// instead of being queued in the snapshot. This matches reference behavior
    /// where InboundLedgers::gotLedgerData is called directly from the
    /// network thread.
    pub fn set_ledger_data_channel(
        &self,
        tx: std::sync::mpsc::SyncSender<PeerMessage<TmLedgerData>>,
    ) {
        *self.ledger_data_tx.lock().expect("ledger_data_tx lock") = Some(tx);
    }

    pub fn clear_ledger_data_channel(&self) {
        *self.ledger_data_tx.lock().expect("ledger_data_tx lock") = None;
    }

    /// Set a direct routing callback for TmLedgerData. When set, this is
    /// called FIRST (before the channel), directly from the network thread.
    /// This eliminates the router thread channel hop for maximum throughput.
    pub fn set_ledger_data_router(
        &self,
        router: Box<dyn Fn(PeerId, TmLedgerData) -> LedgerDataIngressDisposition + Send + Sync>,
    ) {
        let _delivery_gate = self
            .ledger_data_delivery_gate
            .lock()
            .expect("ledger_data_delivery_gate lock");
        tracing::info!(
            target: "consensus",
            handler_ptr = format!("{:p}", self),
            "set_ledger_data_router: SETTING router"
        );
        let router: Arc<
            dyn Fn(PeerId, TmLedgerData) -> LedgerDataIngressDisposition + Send + Sync,
        > = Arc::from(router);
        let queued = {
            // Keep router -> inbound lock ordering consistent with incoming
            // fallback. No packet can enqueue after this drain begins.
            let mut router_guard = self
                .ledger_data_router
                .lock()
                .expect("ledger_data_router lock");
            let mut inbound = self.inner.lock().expect("overlay inbound lock");
            *router_guard = Some(Arc::clone(&router));
            std::mem::take(&mut inbound.ledger_data)
        };
        for packet in queued {
            let _ = router(packet.peer_id, packet.message);
        }
    }

    /// Retry the exactly-one deferred decoded frame for `peer_id`. `true`
    /// keeps the peer's session read-paused; `false` means the frame was
    /// admitted, terminally classified, or no longer exists.
    pub fn retry_deferred_ledger_data(&self, peer_id: PeerId) -> bool {
        let _delivery_gate = self
            .ledger_data_delivery_gate
            .lock()
            .expect("ledger_data_delivery_gate lock");
        let frame = self
            .ledger_data_deferred
            .lock()
            .expect("ledger_data_deferred lock")
            .remove(&peer_id);
        let Some(frame) = frame else {
            return false;
        };
        {
            let mut bytes = self
                .deferred_ledger_data_bytes
                .lock()
                .expect("deferred_ledger_data_bytes lock");
            bytes.current = bytes.current.saturating_sub(frame.reserved_bytes);
        }
        let message = frame.message;
        let router = self
            .ledger_data_router
            .lock()
            .expect("ledger_data_router lock")
            .as_ref()
            .map(Arc::clone);
        let Some(router) = router else {
            // Router teardown is a terminal transport condition. Never retain
            // a decoded frame without a route owner.
            return false;
        };
        if router(peer_id, message.clone()) == LedgerDataIngressDisposition::Deferred {
            let reserved_bytes = deferred_ledger_data_bytes(&message);
            let mut bytes = self
                .deferred_ledger_data_bytes
                .lock()
                .expect("deferred_ledger_data_bytes lock");
            if bytes.current.saturating_add(reserved_bytes) > bytes.limit {
                return false;
            }
            bytes.current += reserved_bytes;
            self.ledger_data_deferred
                .lock()
                .expect("ledger_data_deferred lock")
                .insert(
                    peer_id,
                    DeferredLedgerData {
                        message,
                        reserved_bytes,
                    },
                );
            true
        } else {
            false
        }
    }

    pub fn ledger_data_router_is_set(&self) -> bool {
        self.ledger_data_router
            .lock()
            .expect("ledger_data_router lock")
            .is_some()
    }

    /// Deliver packets that arrived before the direct router was installed.
    ///
    /// The overlay listener can begin receiving messages before bootstrap has
    /// finished wiring the acquisition router. In that window `on_ledger_data`
    /// stores packets in the fallback snapshot queue. Once a router exists,
    /// those packets must be replayed instead of remaining invisible to the
    /// acquisition registry.
    pub fn drain_ledger_data_to_router(&self) -> usize {
        let _delivery_gate = self
            .ledger_data_delivery_gate
            .lock()
            .expect("ledger_data_delivery_gate lock");
        let (router, packets) = {
            let router_guard = self
                .ledger_data_router
                .lock()
                .expect("ledger_data_router lock");
            let mut inbound = self.inner.lock().expect("overlay inbound lock");
            let Some(router) = router_guard.as_ref().map(Arc::clone) else {
                return 0;
            };
            (router, std::mem::take(&mut inbound.ledger_data))
        };
        let count = packets.len();
        for packet in packets {
            let _ = router(packet.peer_id, packet.message);
        }
        count
    }

    pub fn set_proof_path_request_router(
        &self,
        router: Box<dyn Fn(PeerId, TmProofPathRequest) + Send + Sync>,
    ) {
        let router: Arc<dyn Fn(PeerId, TmProofPathRequest) + Send + Sync> = Arc::from(router);
        let queued = {
            let mut router_guard = self
                .proof_path_request_router
                .lock()
                .expect("proof_path_request_router lock");
            let mut inbound = self.inner.lock().expect("overlay inbound lock");
            *router_guard = Some(Arc::clone(&router));
            std::mem::take(&mut inbound.proof_path_requests)
        };
        for request in queued {
            router(request.peer_id, request.message);
        }
    }

    pub fn set_proof_path_response_router(
        &self,
        router: Box<dyn Fn(PeerId, TmProofPathResponse) + Send + Sync>,
    ) {
        let router: Arc<dyn Fn(PeerId, TmProofPathResponse) + Send + Sync> = Arc::from(router);
        let queued = {
            let mut router_guard = self
                .proof_path_response_router
                .lock()
                .expect("proof_path_response_router lock");
            let mut inbound = self.inner.lock().expect("overlay inbound lock");
            *router_guard = Some(Arc::clone(&router));
            std::mem::take(&mut inbound.proof_path_responses)
        };
        for response in queued {
            router(response.peer_id, response.message);
        }
    }

    pub fn set_replay_delta_request_router(
        &self,
        router: Box<dyn Fn(PeerId, TmReplayDeltaRequest) + Send + Sync>,
    ) {
        let router: Arc<dyn Fn(PeerId, TmReplayDeltaRequest) + Send + Sync> = Arc::from(router);
        let queued = {
            let mut router_guard = self
                .replay_delta_request_router
                .lock()
                .expect("replay_delta_request_router lock");
            let mut inbound = self.inner.lock().expect("overlay inbound lock");
            *router_guard = Some(Arc::clone(&router));
            std::mem::take(&mut inbound.replay_delta_requests)
        };
        for request in queued {
            router(request.peer_id, request.message);
        }
    }

    /// Register an immediate replay-delta response route. This mirrors
    /// `PeerImp::onMessage(TMReplayDeltaResponse)` handing valid data to the
    /// application-owned `LedgerReplayMsgHandler`, while retaining packets
    /// received before startup wiring in the fallback queue.
    pub fn set_replay_delta_response_router(
        &self,
        router: Box<dyn Fn(PeerId, TmReplayDeltaResponse) + Send + Sync>,
    ) {
        let router: Arc<dyn Fn(PeerId, TmReplayDeltaResponse) + Send + Sync> = Arc::from(router);
        let queued = {
            let mut router_guard = self
                .replay_delta_response_router
                .lock()
                .expect("replay_delta_response_router lock");
            let mut inbound = self.inner.lock().expect("overlay inbound lock");
            *router_guard = Some(Arc::clone(&router));
            std::mem::take(&mut inbound.replay_delta_responses)
        };
        for response in queued {
            router(response.peer_id, response.message);
        }
    }

    /// Register the immediate transaction-dispatch callback. Matches
    /// reference PeerImp::handleTransaction, which calls
    /// JobQueue::addJob(JtTransaction, "RcvCheckTx", ...) synchronously on
    /// receipt from the network thread, instead of waiting for a timer tick.
    pub fn set_transaction_router(
        &self,
        router: Box<dyn Fn(PeerId, QueuedTransaction) + Send + Sync>,
    ) {
        let router: Arc<dyn Fn(PeerId, QueuedTransaction) + Send + Sync> = Arc::from(router);
        let queued = {
            // Install and drain under router -> inbound locking so an incoming
            // transaction cannot slip between the direct-router check and the
            // fallback enqueue. This mirrors direct JtTransaction handoff once
            // the runtime is available.
            let mut router_guard = self
                .transaction_router
                .lock()
                .expect("transaction_router lock");
            let mut inbound = self.inner.lock().expect("overlay inbound lock");
            *router_guard = Some(Arc::clone(&router));
            std::mem::take(&mut inbound.transactions)
        };
        for transaction in queued {
            router(transaction.peer_id, transaction);
        }
    }

    /// Clear the transaction router so that incoming transactions accumulate
    /// in the queue until a runtime router is installed.
    pub fn clear_transaction_router(&self) {
        *self
            .transaction_router
            .lock()
            .expect("transaction_router lock") = None;
    }

    /// Set a notify callback for when proposals arrive from peers. Called by
    /// the consensus strand setup to get instant wake on proposal arrival,
    /// removing the 50ms poll latency.
    pub fn set_proposal_notify(&self, notify: Box<dyn Fn() + Send + Sync>) {
        *self.proposal_notify.lock().expect("proposal_notify lock") = Some(notify);
    }

    /// Set a direct routing callback for proposals. When set, `on_propose_ledger`
    /// calls this instead of pushing to inner.proposals. This routes proposals
    /// directly to the strand's proposal_tx channel.
    pub fn set_proposal_router(&self, router: Box<dyn Fn(QueuedProposal) + Send + Sync>) {
        let router: Arc<dyn Fn(QueuedProposal) + Send + Sync> = Arc::from(router);
        let queued = {
            let mut router_guard = self.proposal_router.lock().expect("proposal_router lock");
            let mut inbound = self.inner.lock().expect("overlay inbound lock");
            *router_guard = Some(Arc::clone(&router));
            std::mem::take(&mut inbound.proposals)
        };
        for proposal in queued {
            router(proposal);
        }
    }

    /// Set a direct routing callback for GetLedger requests. When set,
    /// `on_get_ledger` calls this instead of pushing to inner.get_ledgers.
    pub fn set_get_ledger_router(&self, router: Box<dyn Fn(PeerId, TmGetLedger) + Send + Sync>) {
        let router: Arc<dyn Fn(PeerId, TmGetLedger) + Send + Sync> = Arc::from(router);
        let queued = {
            let mut router_guard = self
                .get_ledger_router
                .lock()
                .expect("get_ledger_router lock");
            let mut inbound = self.inner.lock().expect("overlay inbound lock");
            *router_guard = Some(Arc::clone(&router));
            std::mem::take(&mut inbound.get_ledgers)
        };
        for request in queued {
            router(request.peer_id, request.message);
        }
    }

    /// Set a direct routing callback for GetObjectByHash requests. When set,
    /// `on_get_objects` calls this instead of pushing to inner.get_objects.
    pub fn set_get_objects_router(
        &self,
        router: Box<dyn Fn(PeerId, TmGetObjectByHash) + Send + Sync>,
    ) {
        let router: Arc<dyn Fn(PeerId, TmGetObjectByHash) + Send + Sync> = Arc::from(router);
        let queued = {
            let mut router_guard = self
                .get_objects_router
                .lock()
                .expect("get_objects_router lock");
            let mut inbound = self.inner.lock().expect("overlay inbound lock");
            *router_guard = Some(Arc::clone(&router));
            std::mem::take(&mut inbound.get_objects)
        };
        for request in queued {
            router(request.peer_id, request.message);
        }
    }

    /// Put validations back into the queue so they can be consumed by the
    /// validation processing loop. Called after take_snapshot() when the
    /// caller only needs ledger_data/get_objects but not validations.
    pub fn requeue_validations(&self, validations: Vec<QueuedValidation>) {
        if validations.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().expect("overlay inbound lock");
        // Validation fallback is used only before runtime routing is installed.
        // Preserve every message in that handoff window; the active runtime
        // uses the direct, lossless router below.
        inner.validations.extend(validations);
    }

    pub fn requeue_proposals(&self, proposals: Vec<QueuedProposal>) {
        if proposals.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().expect("overlay inbound lock");
        extend_bounded(&mut inner.proposals, proposals, "proposals");
    }

    /// Re-queue transactions taken from a snapshot that the caller isn't
    /// consuming itself (e.g. a validation-processor thread that only wants
    /// manifests/validations). Without this, transactions taken via
    /// take_snapshot() and not explicitly handled are silently dropped,
    /// preventing them from ever being applied to the open ledger.
    pub fn requeue_transactions(&self, transactions: Vec<QueuedTransaction>) {
        if transactions.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().expect("overlay inbound lock");
        extend_bounded(&mut inner.transactions, transactions, "transactions");
    }

    /// Drain only validations from the queue, leaving all other messages.
    pub fn take_validations(&self) -> Vec<QueuedValidation> {
        std::mem::take(&mut self.inner.lock().expect("overlay inbound lock").validations)
    }

    /// Set a direct routing callback for validations. When installed this is
    /// the primary path, equivalent to rippled's immediate validation job
    /// dispatch; the callback must retain or backpressure rather than drop.
    pub fn set_validation_router(&self, router: Box<dyn Fn(QueuedValidation) + Send + Sync>) {
        let router: Arc<dyn Fn(QueuedValidation) + Send + Sync> = Arc::from(router);
        let queued = {
            let mut router_guard = self
                .validation_router
                .lock()
                .expect("validation_router lock");
            let mut inbound = self.inner.lock().expect("overlay inbound lock");
            *router_guard = Some(Arc::clone(&router));
            std::mem::take(&mut inbound.validations)
        };
        for validation in queued {
            router(validation);
        }
    }

    /// Register a notify channel for instant validation wake.
    /// The validation processing thread waits on the receiver; when a
    /// validation arrives, this sender fires so the thread wakes immediately.
    pub fn set_validation_notify(&self, tx: std::sync::mpsc::SyncSender<()>) {
        *self
            .validation_notify_tx
            .lock()
            .expect("validation_notify lock") = Some(tx);
    }

    pub fn take_proposals(&self) -> Vec<QueuedProposal> {
        std::mem::take(&mut self.inner.lock().expect("overlay inbound lock").proposals)
    }

    /// Drain only ledger_data from the queue, leaving all other messages.
    pub fn take_ledger_data(&self) -> Vec<PeerMessage<TmLedgerData>> {
        std::mem::take(&mut self.inner.lock().expect("overlay inbound lock").ledger_data)
    }

    /// Drain only manifests from the queue, leaving all other messages
    /// (transactions, proposals, validations, etc.) for their rightful
    /// single consumer. Matches take_validations/take_proposals pattern —
    /// using take_snapshot() here would race with and steal messages meant
    /// for other consumers.
    pub fn take_manifests(&self) -> Vec<PeerMessage<TmManifests>> {
        std::mem::take(&mut self.inner.lock().expect("overlay inbound lock").manifests)
    }

    /// Drain only get_ledger requests from the queue, leaving all other messages.
    pub fn take_get_ledgers(&self) -> Vec<PeerMessage<TmGetLedger>> {
        std::mem::take(&mut self.inner.lock().expect("overlay inbound lock").get_ledgers)
    }

    /// Atomically drain the validator-message families owned by bootstrap.
    ///
    /// A snapshot followed by separate drains can split a v2 collection from
    /// its neighboring manifest or v1 list as ingress continues. Keep these
    /// queues under one lock so every packet is either in this batch or left
    /// intact for the next housekeeping pass.
    pub fn take_validator_messages(
        &self,
    ) -> (
        Vec<PeerMessage<TmManifests>>,
        Vec<PeerMessage<TmValidatorList>>,
        Vec<PeerMessage<TmValidatorListCollection>>,
    ) {
        let mut inbound = self.inner.lock().expect("overlay inbound lock");
        (
            std::mem::take(&mut inbound.manifests),
            std::mem::take(&mut inbound.validator_lists),
            std::mem::take(&mut inbound.validator_list_collections),
        )
    }

    /// Drain only validator list messages from the queue.
    pub fn take_validator_lists(&self) -> Vec<PeerMessage<TmValidatorList>> {
        std::mem::take(
            &mut self
                .inner
                .lock()
                .expect("overlay inbound lock")
                .validator_lists,
        )
    }

    /// Drain only accepted endpoint messages for PeerFinder's livecache.
    pub fn take_endpoints(&self) -> Vec<QueuedEndpoints> {
        std::mem::take(&mut self.inner.lock().expect("overlay inbound lock").endpoints)
    }

    pub fn take_transactions(&self) -> Vec<QueuedTransaction> {
        std::mem::take(
            &mut self
                .inner
                .lock()
                .expect("overlay inbound lock")
                .transactions,
        )
    }

    pub fn transaction_count(&self) -> usize {
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .transactions
            .len()
    }

    pub fn take_get_objects(&self) -> Vec<PeerMessage<TmGetObjectByHash>> {
        std::mem::take(&mut self.inner.lock().expect("overlay inbound lock").get_objects)
    }
}

impl OverlayInboundHandler for QueuedOverlayInboundHandler {
    fn on_manifests(&self, peer: &Arc<PeerImp>, message: TmManifests) {
        let mut inner = self.inner.lock().expect("overlay inbound lock");
        push_bounded(
            &mut inner.manifests,
            PeerMessage {
                peer_id: peer.id(),
                message,
            },
            "manifests",
        );
    }

    fn on_endpoints(&self, _peer: &Arc<PeerImp>, message: QueuedEndpoints) {
        push_bounded(
            &mut self.inner.lock().expect("overlay inbound lock").endpoints,
            message,
            "endpoints",
        );
    }

    fn on_transaction(&self, peer: &Arc<PeerImp>, message: QueuedTransaction) {
        let mut message = Some(message);
        let router = {
            let router_guard = self
                .transaction_router
                .lock()
                .expect("transaction_router lock");
            if let Some(router) = router_guard.as_ref().map(Arc::clone) {
                Some(router)
            } else {
                let mut inbound = self.inner.lock().expect("overlay inbound lock");
                push_bounded(
                    &mut inbound.transactions,
                    message.take().expect("transaction present"),
                    "transactions",
                );
                None
            }
        };
        if let Some(router) = router {
            router(peer.id(), message.take().expect("transaction present"));
        }
    }

    fn on_get_ledger(&self, peer: &Arc<PeerImp>, message: TmGetLedger) {
        let mut message = Some(message);
        let router = self
            .get_ledger_router
            .lock()
            .expect("get_ledger_router lock")
            .as_ref()
            .map(Arc::clone);
        if let Some(router) = router {
            router(peer.id(), message.take().expect("message present"));
            return;
        }

        let router = {
            let router_guard = self
                .get_ledger_router
                .lock()
                .expect("get_ledger_router lock");
            if let Some(router) = router_guard.as_ref().map(Arc::clone) {
                Some(router)
            } else {
                let mut inner = self.inner.lock().expect("overlay inbound lock");
                push_bounded(
                    &mut inner.get_ledgers,
                    PeerMessage {
                        peer_id: peer.id(),
                        message: message.take().expect("message present"),
                    },
                    "get_ledgers",
                );
                None
            }
        };
        if let Some(router) = router {
            router(peer.id(), message.take().expect("message present"));
        }
    }

    fn on_ledger_data(
        &self,
        peer: &Arc<PeerImp>,
        message: TmLedgerData,
    ) -> LedgerDataIngressDisposition {
        let _delivery_gate = self
            .ledger_data_delivery_gate
            .lock()
            .expect("ledger_data_delivery_gate lock");
        let mut message = Some(message);
        let router = self
            .ledger_data_router
            .lock()
            .expect("ledger_data_router lock")
            .as_ref()
            .map(Arc::clone);
        if let Some(router) = router {
            if self
                .ledger_data_deferred
                .lock()
                .expect("ledger_data_deferred lock")
                .contains_key(&peer.id())
            {
                // Keep the one retained frame bounded while allowing control
                // traffic to continue through the session reader.
                tracing::debug!(
                    target: "overlay",
                    peer_id = %peer.id(),
                    "Dropping ledger data while a bounded deferred frame is retained"
                );
                return LedgerDataIngressDisposition::Delivered;
            }
            let message = message.take().expect("message present");
            if router(peer.id(), message.clone()) == LedgerDataIngressDisposition::Deferred {
                return self.defer_ledger_data(peer, message);
            }
            return LedgerDataIngressDisposition::Delivered;
        }

        let router = {
            // Pair this recheck with the setter's router -> inbound locks so
            // fallback packets cannot be stranded after its atomic drain.
            let router_guard = self
                .ledger_data_router
                .lock()
                .expect("ledger_data_router lock");
            if let Some(router) = router_guard.as_ref().map(Arc::clone) {
                Some(router)
            } else {
                let pm = PeerMessage {
                    peer_id: peer.id(),
                    message: message.take().expect("message present"),
                };
                let sent_direct = self
                    .ledger_data_tx
                    .lock()
                    .expect("ledger_data_tx lock")
                    .as_ref()
                    .map(|tx| tx.try_send(pm.clone()).is_ok())
                    .unwrap_or(false);
                if !sent_direct {
                    let mut inbound = self.inner.lock().expect("overlay inbound lock");
                    if !push_bounded(&mut inbound.ledger_data, pm, "ledger_data") {
                        peer.request_disconnect();
                    }
                }
                None
            }
        };
        if let Some(router) = router {
            if self
                .ledger_data_deferred
                .lock()
                .expect("ledger_data_deferred lock")
                .contains_key(&peer.id())
            {
                tracing::debug!(
                    target: "overlay",
                    peer_id = %peer.id(),
                    "Dropping ledger data while a bounded deferred frame is retained"
                );
                return LedgerDataIngressDisposition::Delivered;
            }
            let message = message.take().expect("message present");
            if router(peer.id(), message.clone()) == LedgerDataIngressDisposition::Deferred {
                return self.defer_ledger_data(peer, message);
            }
        }
        LedgerDataIngressDisposition::Delivered
    }

    fn on_propose_ledger(&self, _peer: &Arc<PeerImp>, message: QueuedProposal) {
        let mut message = Some(message);
        let router = self
            .proposal_router
            .lock()
            .expect("proposal_router lock")
            .as_ref()
            .map(Arc::clone);
        if let Some(router) = router {
            router(message.take().expect("message present"));
        } else {
            let router = {
                let router_guard = self.proposal_router.lock().expect("proposal_router lock");
                if let Some(router) = router_guard.as_ref().map(Arc::clone) {
                    Some(router)
                } else {
                    let mut inner = self.inner.lock().expect("overlay inbound lock");
                    push_bounded(
                        &mut inner.proposals,
                        message.take().expect("message present"),
                        "proposals",
                    );
                    None
                }
            };
            if let Some(router) = router {
                router(message.take().expect("message present"));
            }
        }
        // Wake the consensus strand loop immediately.
        if let Ok(notify) = self.proposal_notify.lock()
            && let Some(ref f) = *notify
        {
            f();
        }
    }

    fn on_validation(&self, _peer: &Arc<PeerImp>, message: QueuedValidation) {
        let mut message = Some(message);
        let router = self
            .validation_router
            .lock()
            .expect("validation_router lock")
            .as_ref()
            .map(Arc::clone);
        if let Some(router) = router {
            router(message.take().expect("message present"));
            return;
        }

        let router = {
            let router_guard = self
                .validation_router
                .lock()
                .expect("validation_router lock");
            if let Some(router) = router_guard.as_ref().map(Arc::clone) {
                Some(router)
            } else {
                let mut inner = self.inner.lock().expect("overlay inbound lock");
                inner
                    .validations
                    .push(message.take().expect("message present"));
                None
            }
        };
        if let Some(router) = router {
            router(message.take().expect("message present"));
            return;
        }
        // Wake the validation processing thread immediately.
        if let Some(tx) = self
            .validation_notify_tx
            .lock()
            .expect("validation_notify lock")
            .as_ref()
        {
            let _ = tx.try_send(());
        }
    }

    fn on_validator_list(&self, peer: &Arc<PeerImp>, message: TmValidatorList) {
        let mut inner = self.inner.lock().expect("overlay inbound lock");
        push_bounded(
            &mut inner.validator_lists,
            PeerMessage {
                peer_id: peer.id(),
                message,
            },
            "validator_lists",
        );
    }

    fn on_validator_list_collection(
        &self,
        peer: &Arc<PeerImp>,
        message: TmValidatorListCollection,
    ) {
        let mut inner = self.inner.lock().expect("overlay inbound lock");
        push_bounded(
            &mut inner.validator_list_collections,
            PeerMessage {
                peer_id: peer.id(),
                message,
            },
            "validator_list_collections",
        );
    }

    fn on_get_objects(&self, peer: &Arc<PeerImp>, message: TmGetObjectByHash) {
        let mut message = Some(message);
        let router = self
            .get_objects_router
            .lock()
            .expect("get_objects_router lock")
            .as_ref()
            .map(Arc::clone);
        if let Some(router) = router {
            router(peer.id(), message.take().expect("message present"));
            return;
        }

        let router = {
            let router_guard = self
                .get_objects_router
                .lock()
                .expect("get_objects_router lock");
            if let Some(router) = router_guard.as_ref().map(Arc::clone) {
                Some(router)
            } else {
                let mut inner = self.inner.lock().expect("overlay inbound lock");
                push_bounded(
                    &mut inner.get_objects,
                    PeerMessage {
                        peer_id: peer.id(),
                        message: message.take().expect("message present"),
                    },
                    "get_objects",
                );
                None
            }
        };
        if let Some(router) = router {
            router(peer.id(), message.take().expect("message present"));
        }
    }

    fn on_have_transactions(&self, _peer: &Arc<PeerImp>, message: QueuedHaveTransactions) {
        push_bounded(
            &mut self
                .inner
                .lock()
                .expect("overlay inbound lock")
                .have_transactions,
            message,
            "have_transactions",
        );
    }

    fn on_proof_path_request(&self, peer: &Arc<PeerImp>, message: TmProofPathRequest) {
        if let Some(router) = self
            .proof_path_request_router
            .lock()
            .expect("proof_path_request_router lock")
            .as_ref()
            .map(Arc::clone)
        {
            router(peer.id(), message);
            return;
        }
        let mut inner = self.inner.lock().expect("overlay inbound lock");
        push_bounded(
            &mut inner.proof_path_requests,
            PeerMessage {
                peer_id: peer.id(),
                message,
            },
            "proof_path_requests",
        );
    }

    fn on_proof_path_response(&self, peer: &Arc<PeerImp>, message: TmProofPathResponse) {
        if let Some(router) = self
            .proof_path_response_router
            .lock()
            .expect("proof_path_response_router lock")
            .as_ref()
            .map(Arc::clone)
        {
            router(peer.id(), message);
            return;
        }
        let mut inner = self.inner.lock().expect("overlay inbound lock");
        push_bounded(
            &mut inner.proof_path_responses,
            PeerMessage {
                peer_id: peer.id(),
                message,
            },
            "proof_path_responses",
        );
    }

    fn on_replay_delta_request(&self, peer: &Arc<PeerImp>, message: TmReplayDeltaRequest) {
        if let Some(router) = self
            .replay_delta_request_router
            .lock()
            .expect("replay_delta_request_router lock")
            .as_ref()
            .map(Arc::clone)
        {
            router(peer.id(), message);
            return;
        }
        let mut inner = self.inner.lock().expect("overlay inbound lock");
        push_bounded(
            &mut inner.replay_delta_requests,
            PeerMessage {
                peer_id: peer.id(),
                message,
            },
            "replay_delta_requests",
        );
    }

    fn on_replay_delta_response(&self, peer: &Arc<PeerImp>, message: TmReplayDeltaResponse) {
        let mut message = Some(message);
        let router = {
            let router_guard = self
                .replay_delta_response_router
                .lock()
                .expect("replay_delta_response_router lock");
            if let Some(router) = router_guard.as_ref().map(Arc::clone) {
                Some(router)
            } else {
                let mut inner = self.inner.lock().expect("overlay inbound lock");
                push_bounded(
                    &mut inner.replay_delta_responses,
                    PeerMessage {
                        peer_id: peer.id(),
                        message: message.take().expect("replay delta response present"),
                    },
                    "replay_delta_responses",
                );
                None
            }
        };
        if let Some(router) = router {
            router(
                peer.id(),
                message.take().expect("replay delta response present"),
            );
        }
    }
}

pub fn validator_list_feature_for_message(message: &TmValidatorListCollection) -> ProtocolFeature {
    if message.version >= 2 {
        ProtocolFeature::ValidatorList2Propagation
    } else {
        ProtocolFeature::ValidatorListPropagation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_inbound_family_requeue_preserves_messages() {
        let mut queue = Vec::new();
        extend_bounded(&mut queue, (0..=1_024).collect(), "test");
        assert_eq!(queue.len(), 1_025);
        extend_bounded(&mut queue, vec![1_025usize], "test");
        assert_eq!(queue.len(), 1_026);
    }

    #[test]
    fn additional_ledger_data_is_dropped_while_one_frame_is_deferred() {
        let handler = QueuedOverlayInboundHandler::default();
        handler.set_ledger_data_router(Box::new(|_, _| LedgerDataIngressDisposition::Deferred));
        let public_key = protocol::derive_public_key(
            protocol::KeyType::Secp256k1,
            &protocol::SecretKey::from_bytes([73; 32]),
        )
        .expect("test public key");
        let peer = Arc::new(PeerImp::new(
            73,
            "127.0.0.1:5073".parse().expect("test socket address"),
            public_key,
            "peer-73".to_owned(),
        ));

        assert_eq!(
            handler.on_ledger_data(&peer, TmLedgerData::default()),
            LedgerDataIngressDisposition::Deferred
        );
        let reserved_before = handler.deferred_ledger_data_byte_snapshot().0;
        assert!(reserved_before > 0, "first frame owns the bounded slot");

        assert_eq!(
            handler.on_ledger_data(&peer, TmLedgerData::default()),
            LedgerDataIngressDisposition::Delivered
        );
        assert_eq!(
            handler.deferred_ledger_data_byte_snapshot().0,
            reserved_before,
            "a later frame is not retained beside the first deferred frame"
        );
        assert!(
            !peer.disconnect_requested(),
            "bounded backpressure must not tear down PING/PONG control traffic"
        );
    }

    #[test]
    fn pre_router_messages_replay_to_all_direct_routers() {
        let handler = QueuedOverlayInboundHandler::default();
        let public_key = protocol::derive_public_key(
            protocol::KeyType::Secp256k1,
            &protocol::SecretKey::from_bytes([7; 32]),
        )
        .expect("test public key");
        {
            let mut snapshot = handler.inner.lock().expect("overlay inbound lock");
            snapshot.proposals.push(QueuedProposal {
                peer_id: 11,
                suppression: Uint256::from_u64(1),
                public_key,
                current_tx_hash: Uint256::from_u64(2),
                previous_ledger: Uint256::from_u64(3),
                message: TmProposeSet::default(),
            });
            snapshot.validations.push(QueuedValidation {
                peer_id: 12,
                suppression: Uint256::from_u64(4),
                message: TmValidation::default(),
                validation: None,
            });
            snapshot.get_ledgers.push(PeerMessage {
                peer_id: 13,
                message: TmGetLedger::default(),
            });
            snapshot.ledger_data.push(PeerMessage {
                peer_id: 15,
                message: TmLedgerData::default(),
            });
            snapshot.get_objects.push(PeerMessage {
                peer_id: 14,
                message: TmGetObjectByHash::default(),
            });
            snapshot.transactions.push(QueuedTransaction {
                peer_id: 16,
                id: Uint256::from_u64(5),
                batch: false,
                message: TmTransaction::default(),
            });
            snapshot.proof_path_requests.push(PeerMessage {
                peer_id: 17,
                message: TmProofPathRequest::default(),
            });
            snapshot.proof_path_responses.push(PeerMessage {
                peer_id: 18,
                message: TmProofPathResponse::default(),
            });
            snapshot.replay_delta_requests.push(PeerMessage {
                peer_id: 19,
                message: TmReplayDeltaRequest::default(),
            });
        }

        let proposals = Arc::new(Mutex::new(Vec::new()));
        let validations = Arc::new(Mutex::new(Vec::new()));
        let get_ledgers = Arc::new(Mutex::new(Vec::new()));
        let ledger_data = Arc::new(Mutex::new(Vec::new()));
        let get_objects = Arc::new(Mutex::new(Vec::new()));
        let transactions = Arc::new(Mutex::new(Vec::new()));
        let proof_path_requests = Arc::new(Mutex::new(Vec::new()));
        let proof_path_responses = Arc::new(Mutex::new(Vec::new()));
        let replay_delta_requests = Arc::new(Mutex::new(Vec::new()));
        handler.set_transaction_router(Box::new({
            let received = Arc::clone(&transactions);
            move |peer_id, _| {
                received
                    .lock()
                    .expect("transaction replay lock")
                    .push(peer_id)
            }
        }));
        handler.set_proposal_router(Box::new({
            let received = Arc::clone(&proposals);
            move |proposal| {
                received
                    .lock()
                    .expect("proposal replay lock")
                    .push(proposal.peer_id)
            }
        }));
        handler.set_validation_router(Box::new({
            let received = Arc::clone(&validations);
            move |validation| {
                received
                    .lock()
                    .expect("validation replay lock")
                    .push(validation.peer_id)
            }
        }));
        handler.set_get_ledger_router(Box::new({
            let received = Arc::clone(&get_ledgers);
            move |peer_id, _| {
                received
                    .lock()
                    .expect("get-ledger replay lock")
                    .push(peer_id)
            }
        }));
        handler.set_ledger_data_router(Box::new({
            let received = Arc::clone(&ledger_data);
            move |peer_id, _| {
                received
                    .lock()
                    .expect("ledger-data replay lock")
                    .push(peer_id);
                LedgerDataIngressDisposition::Delivered
            }
        }));
        handler.set_get_objects_router(Box::new({
            let received = Arc::clone(&get_objects);
            move |peer_id, _| {
                received
                    .lock()
                    .expect("get-object replay lock")
                    .push(peer_id)
            }
        }));
        handler.set_proof_path_request_router(Box::new({
            let received = Arc::clone(&proof_path_requests);
            move |peer_id, _| {
                received
                    .lock()
                    .expect("proof request replay lock")
                    .push(peer_id)
            }
        }));
        handler.set_proof_path_response_router(Box::new({
            let received = Arc::clone(&proof_path_responses);
            move |peer_id, _| {
                received
                    .lock()
                    .expect("proof response replay lock")
                    .push(peer_id)
            }
        }));
        handler.set_replay_delta_request_router(Box::new({
            let received = Arc::clone(&replay_delta_requests);
            move |peer_id, _| {
                received
                    .lock()
                    .expect("replay request replay lock")
                    .push(peer_id)
            }
        }));

        assert_eq!(*proposals.lock().expect("proposal replay lock"), vec![11]);
        assert_eq!(
            *validations.lock().expect("validation replay lock"),
            vec![12]
        );
        assert_eq!(
            *get_ledgers.lock().expect("get-ledger replay lock"),
            vec![13]
        );
        assert_eq!(
            *ledger_data.lock().expect("ledger-data replay lock"),
            vec![15]
        );
        assert_eq!(
            *get_objects.lock().expect("get-object replay lock"),
            vec![14]
        );
        assert_eq!(
            *transactions.lock().expect("transaction replay lock"),
            vec![16]
        );
        assert_eq!(
            *proof_path_requests
                .lock()
                .expect("proof request replay lock"),
            vec![17]
        );
        assert_eq!(
            *proof_path_responses
                .lock()
                .expect("proof response replay lock"),
            vec![18]
        );
        assert_eq!(
            *replay_delta_requests
                .lock()
                .expect("replay request replay lock"),
            vec![19]
        );
        let snapshot = handler.snapshot();
        assert!(snapshot.proposals.is_empty());
        assert!(snapshot.validations.is_empty());
        assert!(snapshot.get_ledgers.is_empty());
        assert!(snapshot.ledger_data.is_empty());
        assert!(snapshot.get_objects.is_empty());
        assert!(snapshot.transactions.is_empty());
    }

    #[test]
    fn validator_message_drain_is_atomic_and_preserves_later_ingress() {
        let handler = QueuedOverlayInboundHandler::default();
        {
            let mut snapshot = handler.inner.lock().expect("overlay inbound lock");
            snapshot.manifests.push(PeerMessage {
                peer_id: 1,
                message: TmManifests::default(),
            });
            snapshot.validator_lists.push(PeerMessage {
                peer_id: 2,
                message: TmValidatorList::default(),
            });
            snapshot.validator_list_collections.push(PeerMessage {
                peer_id: 3,
                message: TmValidatorListCollection::default(),
            });
        }

        let (manifests, lists, collections) = handler.take_validator_messages();
        assert_eq!(
            manifests
                .iter()
                .map(|message| message.peer_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(
            lists
                .iter()
                .map(|message| message.peer_id)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(
            collections
                .iter()
                .map(|message| message.peer_id)
                .collect::<Vec<_>>(),
            vec![3]
        );

        handler
            .inner
            .lock()
            .expect("overlay inbound lock")
            .validator_list_collections
            .push(PeerMessage {
                peer_id: 4,
                message: TmValidatorListCollection::default(),
            });
        let (_, lists, collections) = handler.take_validator_messages();
        assert!(lists.is_empty());
        assert_eq!(
            collections
                .iter()
                .map(|message| message.peer_id)
                .collect::<Vec<_>>(),
            vec![4]
        );
    }

    #[test]
    fn validation_requeue_preserves_messages() {
        let handler = QueuedOverlayInboundHandler::default();
        let validation = QueuedValidation {
            peer_id: 1,
            suppression: Uint256::zero(),
            message: TmValidation::default(),
            validation: None,
        };
        handler.requeue_validations(vec![validation; 1_025]);
        assert_eq!(handler.snapshot().validations.len(), 1_025);

        handler.requeue_validations(vec![QueuedValidation {
            peer_id: 2,
            suppression: Uint256::zero(),
            message: TmValidation::default(),
            validation: None,
        }]);
        assert_eq!(handler.take_validations().len(), 1_026);
    }
}

#[test]
fn replay_and_proof_fallbacks_replay_to_direct_routes() {
    // LedgerReplayMsgHandler.cpp processes these four message families on
    // receipt. The Rust startup queue may delay them briefly, but must
    // drain every packet once the equivalent app callback is installed.
    let handler = QueuedOverlayInboundHandler::default();
    {
        let mut snapshot = handler.inner.lock().expect("overlay inbound lock");
        snapshot.proof_path_requests.push(PeerMessage {
            peer_id: 21,
            message: TmProofPathRequest {
                key: Uint256::from_u64(1).data().to_vec(),
                ledger_hash: Uint256::from_u64(2).data().to_vec(),
                r#type: 2,
            },
        });
        snapshot.proof_path_responses.push(PeerMessage {
            peer_id: 22,
            message: TmProofPathResponse {
                key: Uint256::from_u64(3).data().to_vec(),
                ledger_hash: Uint256::from_u64(4).data().to_vec(),
                r#type: 2,
                ledger_header: Some(vec![5]),
                path: vec![vec![6]],
                error: None,
            },
        });
        snapshot.replay_delta_requests.push(PeerMessage {
            peer_id: 23,
            message: TmReplayDeltaRequest {
                ledger_hash: Uint256::from_u64(7).data().to_vec(),
            },
        });
        snapshot.replay_delta_responses.push(PeerMessage {
            peer_id: 24,
            message: TmReplayDeltaResponse {
                ledger_hash: Uint256::from_u64(8).data().to_vec(),
                ledger_header: None,
                transaction: Vec::new(),
                error: Some(1),
            },
        });
    }

    let received = Arc::new(Mutex::new(Vec::new()));
    handler.set_proof_path_request_router(Box::new({
        let received = Arc::clone(&received);
        move |peer_id, _| received.lock().expect("received lock").push(peer_id)
    }));
    handler.set_proof_path_response_router(Box::new({
        let received = Arc::clone(&received);
        move |peer_id, _| received.lock().expect("received lock").push(peer_id)
    }));
    handler.set_replay_delta_request_router(Box::new({
        let received = Arc::clone(&received);
        move |peer_id, _| received.lock().expect("received lock").push(peer_id)
    }));
    handler.set_replay_delta_response_router(Box::new({
        let received = Arc::clone(&received);
        move |peer_id, _| received.lock().expect("received lock").push(peer_id)
    }));

    assert_eq!(
        *received.lock().expect("received lock"),
        vec![21, 22, 23, 24]
    );
    let snapshot = handler.snapshot();
    assert!(snapshot.proof_path_requests.is_empty());
    assert!(snapshot.proof_path_responses.is_empty());
    assert!(snapshot.replay_delta_requests.is_empty());
    assert!(snapshot.replay_delta_responses.is_empty());
}
