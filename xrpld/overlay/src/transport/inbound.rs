//! Explicit inbound overlay family seams above the wire codec.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use basics::base_uint::Uint256;
use protocol::PublicKey;

use crate::message::{
    TmEndpoints, TmGetLedger, TmGetObjectByHash, TmHaveTransactions, TmLedgerData, TmManifests,
    TmProofPathRequest, TmProofPathResponse, TmProposeSet, TmReplayDeltaRequest,
    TmReplayDeltaResponse, TmTransaction, TmTransactions, TmValidation, TmValidatorList,
    TmValidatorListCollection,
};
use crate::peer::{Peer, PeerId, ProtocolFeature};
use crate::peer_imp::PeerImp;

#[derive(Debug, Clone, PartialEq)]
pub struct PeerMessage<T> {
    pub peer_id: PeerId,
    pub message: T,
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

#[derive(Debug, Clone, PartialEq)]
pub struct QueuedValidation {
    pub peer_id: PeerId,
    pub suppression: Uint256,
    pub message: TmValidation,
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
    pub transactions_batches: Vec<PeerMessage<TmTransactions>>,
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
    fn on_ledger_data(&self, _peer: &Arc<PeerImp>, _message: TmLedgerData) {}
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
    fn on_transactions(&self, _peer: &Arc<PeerImp>, _message: TmTransactions) {}
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
    ledger_data_tx: Mutex<Option<std::sync::mpsc::Sender<PeerMessage<TmLedgerData>>>>,
    /// Direct routing callback for TmLedgerData — routes immediately to
    /// acquisition threads without any channel hop. This is the fastest path,
    /// matching reference where gotLedgerData dispatches directly from the network thread.
    #[allow(clippy::type_complexity)]
    ledger_data_router: Mutex<Option<Box<dyn Fn(PeerId, TmLedgerData) + Send + Sync>>>,
    /// Direct routing callback for inbound transactions — dispatches
    /// immediately to a JobQueue worker on receipt, matching reference
    /// PeerImp::handleTransaction -> JobQueue::addJob(JtTransaction,
    /// "RcvCheckTx", ...). Without this, transactions only got processed on
    /// the next 1s overlay timer tick, which is too slow relative to
    /// consensus round-close timing and causes sporadic quorum misses.
    #[allow(clippy::type_complexity)]
    transaction_router: Mutex<Option<Box<dyn Fn(PeerId, QueuedTransaction) + Send + Sync>>>,
    /// Notify channel for instant validation wake. When a validation arrives,
    /// a signal is sent so the validation processing thread wakes immediately
    /// instead of polling every 500ms. Matches reference where validations trigger
    /// checkAccept synchronously on the network thread.
    validation_notify_tx: Mutex<Option<std::sync::mpsc::SyncSender<()>>>,
    /// Notify callback for instant batch-thread wake when a relay transaction
    /// arrives and no router is set. Matches rippled's doTransactionAsync
    /// scheduling a JtBatch job on first arrival.
    transaction_notify: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    /// Notify callback to wake the consensus strand loop immediately when a
    /// proposal arrives. Removes the 50ms poll latency, matching rippled's
    /// strand-based immediate dispatch of proposals.
    proposal_notify: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    /// Direct routing callback for proposals — sends directly to the strand's
    /// proposal_tx channel instead of queuing. Matches rippled's event-driven
    /// model where proposals are dispatched immediately to the strand.
    #[allow(clippy::type_complexity)]
    proposal_router: Mutex<Option<Box<dyn Fn(QueuedProposal) + Send + Sync>>>,
    /// Direct routing callback for GetLedger requests — dispatches directly
    /// to the JobQueue instead of queuing for the polling loop.
    #[allow(clippy::type_complexity)]
    get_ledger_router: Mutex<Option<Box<dyn Fn(PeerId, TmGetLedger) + Send + Sync>>>,
    /// Direct routing callback for GetObjectByHash requests — dispatches
    /// directly to the JobQueue instead of queuing for the polling loop.
    #[allow(clippy::type_complexity)]
    get_objects_router: Mutex<Option<Box<dyn Fn(PeerId, TmGetObjectByHash) + Send + Sync>>>,
}

impl Default for QueuedOverlayInboundHandler {
    fn default() -> Self {
        Self {
            inner: Mutex::new(OverlayInboundSnapshot::default()),
            ledger_data_tx: Mutex::new(None),
            ledger_data_router: Mutex::new(None),
            transaction_router: Mutex::new(None),
            validation_notify_tx: Mutex::new(None),
            transaction_notify: Mutex::new(None),
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

    pub fn clear(&self) {
        *self.inner.lock().expect("overlay inbound lock") = OverlayInboundSnapshot::default();
    }

    /// Register a channel for immediate TmLedgerData delivery.
    /// When set, TmLedgerData messages are sent to this channel immediately
    /// instead of being queued in the snapshot. This matches reference behavior
    /// where InboundLedgers::gotLedgerData is called directly from the
    /// network thread.
    pub fn set_ledger_data_channel(&self, tx: std::sync::mpsc::Sender<PeerMessage<TmLedgerData>>) {
        *self.ledger_data_tx.lock().expect("ledger_data_tx lock") = Some(tx);
    }

    pub fn clear_ledger_data_channel(&self) {
        *self.ledger_data_tx.lock().expect("ledger_data_tx lock") = None;
    }

    /// Set a direct routing callback for TmLedgerData. When set, this is
    /// called FIRST (before the channel), directly from the network thread.
    /// This eliminates the router thread channel hop for maximum throughput.
    pub fn set_ledger_data_router(&self, router: Box<dyn Fn(PeerId, TmLedgerData) + Send + Sync>) {
        tracing::info!(target: "consensus", handler_ptr = format!("{:p}", self), "set_ledger_data_router: SETTING router");
        *self
            .ledger_data_router
            .lock()
            .expect("ledger_data_router lock") = Some(router);
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
        let packets = self.take_ledger_data();
        if packets.is_empty() {
            return 0;
        }

        let router = self
            .ledger_data_router
            .lock()
            .expect("ledger_data_router lock");
        let Some(router) = router.as_ref() else {
            self.inner
                .lock()
                .expect("overlay inbound lock")
                .ledger_data
                .extend(packets);
            return 0;
        };

        let count = packets.len();
        for packet in packets {
            router(packet.peer_id, packet.message);
        }
        count
    }

    /// Register the immediate transaction-dispatch callback. Matches
    /// reference PeerImp::handleTransaction, which calls
    /// JobQueue::addJob(JtTransaction, "RcvCheckTx", ...) synchronously on
    /// receipt from the network thread, instead of waiting for a timer tick.
    pub fn set_transaction_router(&self, router: Box<dyn Fn(PeerId, QueuedTransaction) + Send + Sync>) {
        *self
            .transaction_router
            .lock()
            .expect("transaction_router lock") = Some(router);
    }

    /// Clear the transaction router so that incoming transactions accumulate
    /// in the queue (retrieved via `take_transactions`). Used when consensus
    /// starts and the bootstrap loop takes over transaction processing.
    pub fn clear_transaction_router(&self) {
        *self
            .transaction_router
            .lock()
            .expect("transaction_router lock") = None;
    }

    /// Set a notify callback for when relay transactions are queued (no router set).
    /// Called by the batch-apply thread setup to get instant wake on relay arrival.
    pub fn set_transaction_notify(&self, notify: Box<dyn Fn() + Send + Sync>) {
        *self.transaction_notify.lock().expect("transaction_notify lock") = Some(notify);
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
        *self.proposal_router.lock().expect("proposal_router lock") = Some(router);
    }

    /// Set a direct routing callback for GetLedger requests. When set,
    /// `on_get_ledger` calls this instead of pushing to inner.get_ledgers.
    pub fn set_get_ledger_router(&self, router: Box<dyn Fn(PeerId, TmGetLedger) + Send + Sync>) {
        *self.get_ledger_router.lock().expect("get_ledger_router lock") = Some(router);
    }

    /// Set a direct routing callback for GetObjectByHash requests. When set,
    /// `on_get_objects` calls this instead of pushing to inner.get_objects.
    pub fn set_get_objects_router(&self, router: Box<dyn Fn(PeerId, TmGetObjectByHash) + Send + Sync>) {
        *self.get_objects_router.lock().expect("get_objects_router lock") = Some(router);
    }

    /// Put validations back into the queue so they can be consumed by the
    /// validation processing loop. Called after take_snapshot() when the
    /// caller only needs ledger_data/get_objects but not validations.
    pub fn requeue_validations(&self, validations: Vec<QueuedValidation>) {
        if validations.is_empty() {
            return;
        }
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .validations
            .extend(validations);
    }

    pub fn requeue_proposals(&self, proposals: Vec<QueuedProposal>) {
        if proposals.is_empty() {
            return;
        }
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .proposals
            .extend(proposals);
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
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .transactions
            .extend(transactions);
    }

    /// Drain only validations from the queue, leaving all other messages.
    pub fn take_validations(&self) -> Vec<QueuedValidation> {
        std::mem::take(&mut self.inner.lock().expect("overlay inbound lock").validations)
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

    /// Drain only validator list messages from the queue.
    pub fn take_validator_lists(&self) -> Vec<PeerMessage<TmValidatorList>> {
        std::mem::take(&mut self.inner.lock().expect("overlay inbound lock").validator_lists)
    }

    pub fn take_transactions(&self) -> Vec<QueuedTransaction> {
        std::mem::take(&mut self.inner.lock().expect("overlay inbound lock").transactions)
    }

    pub fn transaction_count(&self) -> usize {
        self.inner.lock().expect("overlay inbound lock").transactions.len()
    }

    pub fn take_get_objects(&self) -> Vec<PeerMessage<TmGetObjectByHash>> {
        std::mem::take(&mut self.inner.lock().expect("overlay inbound lock").get_objects)
    }
}

impl OverlayInboundHandler for QueuedOverlayInboundHandler {
    fn on_manifests(&self, peer: &Arc<PeerImp>, message: TmManifests) {
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .manifests
            .push(PeerMessage {
                peer_id: peer.id(),
                message,
            });
    }

    fn on_endpoints(&self, _peer: &Arc<PeerImp>, message: QueuedEndpoints) {
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .endpoints
            .push(message);
    }

    fn on_transaction(&self, peer: &Arc<PeerImp>, message: QueuedTransaction) {
        let router_guard = self.transaction_router.lock().expect("transaction_router lock");
        if let Some(router) = router_guard.as_ref() {
            router(peer.id(), message);
            return;
        }
        drop(router_guard);
        let mut guard = self.inner.lock().expect("overlay inbound lock");
        guard.transactions.push(message);
        drop(guard);
        // Wake the batch-apply thread immediately (matches rippled's
        // doTransactionAsync scheduling JtBatch on first arrival)
        if let Ok(notify) = self.transaction_notify.lock() {
            if let Some(ref f) = *notify {
                f();
            }
        }
    }

    fn on_get_ledger(&self, peer: &Arc<PeerImp>, message: TmGetLedger) {
        // Try direct router first — dispatches to JobQueue immediately
        {
            let guard = self.get_ledger_router.lock().expect("get_ledger_router lock");
            if let Some(router) = guard.as_ref() {
                router(peer.id(), message);
                return;
            }
        }
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .get_ledgers
            .push(PeerMessage {
                peer_id: peer.id(),
                message,
            });
    }

    fn on_ledger_data(&self, peer: &Arc<PeerImp>, message: TmLedgerData) {
        if message.r#type == 3 {
            tracing::info!(target: "consensus", handler_ptr = format!("{:p}", self), peer_id = peer.id(), "on_ledger_data: type=3 ARRIVED");
        }
        // Try direct router callback first — zero channel hops, called
        // directly from the network thread matching reference gotLedgerData.
        {
            let guard = self
                .ledger_data_router
                .lock()
                .expect("ledger_data_router lock");
            if let Some(router) = guard.as_ref() {
                router(peer.id(), message.clone());
                return;
            }
        }
        let pm = PeerMessage {
            peer_id: peer.id(),
            message,
        };
        // Try direct channel (one channel hop)
        let sent_direct = self
            .ledger_data_tx
            .lock()
            .expect("ledger_data_tx lock")
            .as_ref()
            .map(|tx| tx.send(pm.clone()).is_ok())
            .unwrap_or(false);
        // Fallback to snapshot queue
        if !sent_direct {
            self.inner
                .lock()
                .expect("overlay inbound lock")
                .ledger_data
                .push(pm);
        }
    }

    fn on_propose_ledger(&self, _peer: &Arc<PeerImp>, message: QueuedProposal) {
        // Try direct router first — routes to strand's proposal_tx channel
        {
            let guard = self.proposal_router.lock().expect("proposal_router lock");
            if let Some(router) = guard.as_ref() {
                router(message);
                // Still fire the notify so the strand wakes immediately
                if let Ok(notify) = self.proposal_notify.lock() {
                    if let Some(ref f) = *notify {
                        f();
                    }
                }
                return;
            }
        }
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .proposals
            .push(message);
        // Wake the consensus strand loop immediately.
        if let Ok(notify) = self.proposal_notify.lock() {
            if let Some(ref f) = *notify {
                f();
            }
        }
    }

    fn on_validation(&self, _peer: &Arc<PeerImp>, message: QueuedValidation) {
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .validations
            .push(message);
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
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .validator_lists
            .push(PeerMessage {
                peer_id: peer.id(),
                message,
            });
    }

    fn on_validator_list_collection(
        &self,
        peer: &Arc<PeerImp>,
        message: TmValidatorListCollection,
    ) {
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .validator_list_collections
            .push(PeerMessage {
                peer_id: peer.id(),
                message,
            });
    }

    fn on_get_objects(&self, peer: &Arc<PeerImp>, message: TmGetObjectByHash) {
        // Try direct router first — dispatches to JobQueue immediately
        {
            let guard = self.get_objects_router.lock().expect("get_objects_router lock");
            if let Some(router) = guard.as_ref() {
                router(peer.id(), message);
                return;
            }
        }
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .get_objects
            .push(PeerMessage {
                peer_id: peer.id(),
                message,
            });
    }

    fn on_have_transactions(&self, _peer: &Arc<PeerImp>, message: QueuedHaveTransactions) {
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .have_transactions
            .push(message);
    }

    fn on_transactions(&self, peer: &Arc<PeerImp>, message: TmTransactions) {
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .transactions_batches
            .push(PeerMessage {
                peer_id: peer.id(),
                message,
            });
    }

    fn on_proof_path_request(&self, peer: &Arc<PeerImp>, message: TmProofPathRequest) {
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .proof_path_requests
            .push(PeerMessage {
                peer_id: peer.id(),
                message,
            });
    }

    fn on_proof_path_response(&self, peer: &Arc<PeerImp>, message: TmProofPathResponse) {
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .proof_path_responses
            .push(PeerMessage {
                peer_id: peer.id(),
                message,
            });
    }

    fn on_replay_delta_request(&self, peer: &Arc<PeerImp>, message: TmReplayDeltaRequest) {
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .replay_delta_requests
            .push(PeerMessage {
                peer_id: peer.id(),
                message,
            });
    }

    fn on_replay_delta_response(&self, peer: &Arc<PeerImp>, message: TmReplayDeltaResponse) {
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .replay_delta_responses
            .push(PeerMessage {
                peer_id: peer.id(),
                message,
            });
    }
}

pub fn validator_list_feature_for_message(message: &TmValidatorListCollection) -> ProtocolFeature {
    if message.version >= 2 {
        ProtocolFeature::ValidatorList2Propagation
    } else {
        ProtocolFeature::ValidatorListPropagation
    }
}
