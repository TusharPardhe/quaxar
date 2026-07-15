//! `TransactionAcquire` owner port above the landed Rust `SyncTree` and
//! overlay peer-set seams.

use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use overlay::{Peer, PeerSet, ProtocolMessage, ProtocolPayload, TmGetLedger};
use shamap::family::NullFullBelowCache;
use shamap::fetch::SHAMapSyncFilter;
use shamap::node_id::{SHAMapNodeId, deserialize_shamap_node_id};
use shamap::sync::{SHAMapAddNode, SHAMapType, SyncTree};
use std::sync::Arc;

const LI_TS_CANDIDATE: i32 = 3;
const QT_INDIRECT: i32 = 0;
const QUERY_DEPTH: u32 = 3;
pub const TX_ACQUIRE_NORM_TIMEOUTS: i32 = 4;
pub const TX_ACQUIRE_MAX_TIMEOUTS: i32 = 20;

pub trait TransactionAcquireFilterFactory: Send + Sync {
    fn build_filter(&self) -> Box<dyn SHAMapSyncFilter>;
}

pub struct TransactionAcquire {
    hash: Uint256,
    map: SyncTree,
    have_root: bool,
    filter_factory: Option<Arc<dyn TransactionAcquireFilterFactory>>,
    peer_set: Arc<dyn PeerSet>,
    complete: bool,
    failed: bool,
    stopping: bool,
    progress: bool,
    timeouts: i32,
}

impl TransactionAcquire {
    pub fn new(hash: Uint256, peer_set: Arc<dyn PeerSet>) -> Self {
        Self::with_filter_factory(hash, peer_set, None)
    }

    pub fn with_filter_factory(
        hash: Uint256,
        peer_set: Arc<dyn PeerSet>,
        filter_factory: Option<Arc<dyn TransactionAcquireFilterFactory>>,
    ) -> Self {
        let mut map = SyncTree::new_synching_with_type(SHAMapType::Transaction, true, 0);
        map.set_unbacked();
        Self {
            hash,
            map,
            have_root: false,
            filter_factory,
            peer_set,
            complete: false,
            failed: false,
            stopping: false,
            progress: false,
            timeouts: 0,
        }
    }

    pub fn hash(&self) -> Uint256 {
        self.hash
    }

    pub fn map(&self) -> &SyncTree {
        &self.map
    }

    pub fn is_done(&self) -> bool {
        self.complete || self.failed || self.stopping
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn is_failed(&self) -> bool {
        self.failed
    }

    pub fn is_stopped(&self) -> bool {
        self.stopping
    }

    pub fn has_root(&self) -> bool {
        self.have_root
    }

    pub fn timeouts(&self) -> i32 {
        self.timeouts
    }

    pub fn init(&mut self, start_peers: usize) {
        if self.is_done() {
            return;
        }
        self.add_peers(start_peers);
    }

    pub fn still_need(&mut self) {
        if self.stopping {
            return;
        }
        self.timeouts = self.timeouts.min(TX_ACQUIRE_NORM_TIMEOUTS);
        self.failed = false;
    }

    pub fn stop(&mut self) {
        self.stopping = true;
    }

    pub fn invoke_on_timer(&mut self) {
        if self.is_done() {
            return;
        }

        if !self.progress {
            self.timeouts += 1;
            self.on_timer(false);
        } else {
            self.progress = false;
            self.on_timer(true);
        }
    }

    pub fn take_nodes(
        &mut self,
        data: &[(SHAMapNodeId, Vec<u8>)],
        peer: Option<Arc<dyn Peer>>,
    ) -> SHAMapAddNode {
        if self.is_done() {
            return SHAMapAddNode::default();
        }

        if data.is_empty() {
            return SHAMapAddNode::invalid();
        }

        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;
        let mut full_below = NullFullBelowCache::new(1);
        let mut fetch = |_| None;

        for (node_id, raw_node) in data {
            if node_id.is_root() {
                if self.have_root {
                    continue;
                }
                if self
                    .map
                    .add_root_node(SHAMapHash::new(self.hash), raw_node, &mut no_filter)
                    .is_good()
                {
                    self.have_root = true;
                }
            } else {
                let accepted = Self::with_built_filter(self.filter_factory.clone(), |filter| {
                    self.map
                        .add_known_node(*node_id, raw_node, filter, &mut full_below, &mut fetch)
                });
                if !accepted.is_good() {
                    return SHAMapAddNode::invalid();
                }
            }
        }

        self.trigger(peer);
        self.progress = true;
        SHAMapAddNode::useful()
    }

    pub fn take_ledger_data(
        &mut self,
        packet: &overlay::TmLedgerData,
        peer: Option<Arc<dyn Peer>>,
    ) -> TransactionAcquireDataResult {
        if self.is_done() {
            return TransactionAcquireDataResult::Applied(SHAMapAddNode::default());
        }

        let mut data = Vec::with_capacity(packet.nodes.len());

        for node in &packet.nodes {
            let Some(node_id) = node.nodeid.as_ref() else {
                return TransactionAcquireDataResult::MissingNodeId;
            };
            let Some(parsed) = deserialize_shamap_node_id(node_id) else {
                return TransactionAcquireDataResult::InvalidNodeId;
            };
            data.push((parsed, node.nodedata.clone()));
        }

        TransactionAcquireDataResult::Applied(self.take_nodes(&data, peer))
    }

    fn on_timer(&mut self, _progress: bool) {
        if self.timeouts > TX_ACQUIRE_MAX_TIMEOUTS {
            self.failed = true;
            return;
        }

        if self.timeouts >= TX_ACQUIRE_NORM_TIMEOUTS {
            self.trigger(None);
        }

        self.add_peers(1);
    }

    fn trigger(&mut self, peer: Option<Arc<dyn Peer>>) {
        if self.is_done() {
            return;
        }

        if !self.have_root {
            let request = ProtocolMessage::new(ProtocolPayload::GetLedger(TmGetLedger {
                itype: LI_TS_CANDIDATE,
                ltype: None,
                ledger_hash: Some(self.hash.data().to_vec()),
                ledger_seq: None,
                node_i_ds: vec![SHAMapNodeId::default().get_raw_string()],
                request_cookie: None,
                query_type: (self.timeouts != 0).then_some(QT_INDIRECT),
                query_depth: Some(QUERY_DEPTH),
            }));
            // Broadcast the root request to ALL peers (pass None to
            // send_request which triggers the broadcast path). This ensures
            // the request reaches the one peer that actually has the set,
            // even if TMHaveTransactionSet hasn't been processed yet due to
            // message ordering. For a 5-node cluster this is just 4 extra
            // messages per acquisition — negligible overhead, and critical
            // for dispute resolution to complete within the round time.
            self.send_request(&request, None);
            return;
        }

        if !self.map.is_valid() {
            self.failed = true;
            return;
        }

        let mut full_below = NullFullBelowCache::new(1);
        let mut fetch = |_| None;
        let mut next_first_child = || 0;
        let missing = Self::with_built_filter(self.filter_factory.clone(), |filter| {
            self.map.get_missing_nodes(
                256,
                filter,
                &mut full_below,
                &mut fetch,
                &mut next_first_child,
            )
        });

        if missing.is_empty() {
            if self.map.is_valid() {
                self.complete = true;
            } else {
                self.failed = true;
            }
            return;
        }

        let request = ProtocolMessage::new(ProtocolPayload::GetLedger(TmGetLedger {
            itype: LI_TS_CANDIDATE,
            ltype: None,
            ledger_hash: Some(self.hash.data().to_vec()),
            ledger_seq: None,
            node_i_ds: missing
                .into_iter()
                .map(|(node_id, _)| node_id.get_raw_string())
                .collect(),
            request_cookie: None,
            query_type: (self.timeouts != 0).then_some(QT_INDIRECT),
            query_depth: None,
        }));
        self.send_request(&request, peer);
    }

    fn add_peers(&mut self, limit: usize) {
        if self.is_done() {
            return;
        }

        let mut peers = Vec::new();
        self.peer_set
            .add_peers(limit, &mut |peer| peer.has_tx_set(self.hash), &mut |peer| {
                peers.push(Arc::clone(peer))
            });

        for peer in peers {
            self.trigger(Some(peer));
        }
    }

    fn send_request(&self, request: &ProtocolMessage, peer: Option<Arc<dyn Peer>>) {
        if self.is_done() {
            return;
        }

        if let Some(peer) = peer {
            self.peer_set.send_request(request, Some(&peer));
            return;
        }

        // Broadcast to all peers via the PeerSet's None path.
        self.peer_set.send_request(request, None);
    }

    fn with_built_filter<T>(
        filter_factory: Option<Arc<dyn TransactionAcquireFilterFactory>>,
        apply: impl FnOnce(&mut Option<&mut dyn SHAMapSyncFilter>) -> T,
    ) -> T {
        let mut owned_filter = filter_factory.map(|factory| factory.build_filter());
        let mut filter: Option<&mut dyn SHAMapSyncFilter> = match owned_filter.as_mut() {
            Some(filter) => Some(filter.as_mut()),
            None => None,
        };
        apply(&mut filter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionAcquireDataResult {
    MissingNodeId,
    InvalidNodeId,
    Applied(SHAMapAddNode),
}
