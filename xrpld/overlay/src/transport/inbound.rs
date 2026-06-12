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
    /// Notify channel for instant validation wake. When a validation arrives,
    /// a signal is sent so the validation processing thread wakes immediately
    /// instead of polling every 500ms. Matches reference where validations trigger
    /// checkAccept synchronously on the network thread.
    validation_notify_tx: Mutex<Option<std::sync::mpsc::SyncSender<()>>>,
}

impl Default for QueuedOverlayInboundHandler {
    fn default() -> Self {
        Self {
            inner: Mutex::new(OverlayInboundSnapshot::default()),
            ledger_data_tx: Mutex::new(None),
            ledger_data_router: Mutex::new(None),
            validation_notify_tx: Mutex::new(None),
        }
    }
}

impl QueuedOverlayInboundHandler {
    pub fn snapshot(&self) -> OverlayInboundSnapshot {
        self.inner.lock().expect("overlay inbound lock").clone()
    }

    pub fn take_snapshot(&self) -> OverlayInboundSnapshot {
        let mut guard = self.inner.lock().expect("overlay inbound lock");
        std::mem::take(&mut *guard)
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
        *self
            .ledger_data_router
            .lock()
            .expect("ledger_data_router lock") = Some(router);
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

    /// Drain only get_ledger requests from the queue, leaving all other messages.
    pub fn take_get_ledgers(&self) -> Vec<PeerMessage<TmGetLedger>> {
        std::mem::take(&mut self.inner.lock().expect("overlay inbound lock").get_ledgers)
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

    fn on_transaction(&self, _peer: &Arc<PeerImp>, message: QueuedTransaction) {
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .transactions
            .push(message);
    }

    fn on_get_ledger(&self, peer: &Arc<PeerImp>, message: TmGetLedger) {
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
        self.inner
            .lock()
            .expect("overlay inbound lock")
            .proposals
            .push(message);
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
