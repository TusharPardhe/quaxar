//! Peer-set support for targeted peer queries.

use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use crate::message::ProtocolMessage;
use crate::peer::{Peer, PeerId};

pub trait PeerSet: Send + Sync {
    fn add_peers(
        &self,
        limit: usize,
        has_item: &mut dyn FnMut(&Arc<dyn Peer>) -> bool,
        on_peer_added: &mut dyn FnMut(&Arc<dyn Peer>),
    );
    /// Send to a specific peer, or broadcast to all tracked peers if peer is None.
    /// Matches reference PeerSet::sendRequest(message, peer) where peer=nullptr broadcasts.
    fn send_request(&self, message: &ProtocolMessage, peer: Option<&Arc<dyn Peer>>);
    fn peer_ids(&self) -> BTreeSet<PeerId>;
    fn peer_count(&self) -> usize;
}

pub trait PeerSetBuilder: Send + Sync {
    fn build(&self) -> Arc<dyn PeerSet>;
}

#[derive(Default)]
pub struct SimplePeerSet {
    peers: Mutex<VecDeque<Arc<dyn Peer>>>,
    peer_ids: Mutex<BTreeSet<PeerId>>,
}

impl SimplePeerSet {
    pub fn new(peers: impl IntoIterator<Item = Arc<dyn Peer>>) -> Self {
        let peers = peers.into_iter().collect::<VecDeque<_>>();
        Self {
            peers: Mutex::new(peers),
            peer_ids: Mutex::new(BTreeSet::new()),
        }
    }

    /// Refresh the available peer list (reference overlay.foreach gets fresh peers each call).
    pub fn refresh_peers(&self, peers: impl IntoIterator<Item = Arc<dyn Peer>>) {
        let mut guard = self.peers.lock().expect("peer set lock");
        *guard = peers.into_iter().collect();
    }

    /// Find a tracked peer by ID. Returns None if the peer is not in the tracked set.
    pub fn find_peer(&self, id: PeerId) -> Option<Arc<dyn Peer>> {
        // Keep the availability and membership mutexes independent. In
        // particular, add_peers scores an availability snapshot before it
        // updates membership, so neither path may hold one mutex while waiting
        // for the other.
        let is_tracked = {
            let peer_ids = self.peer_ids.lock().expect("peer ids lock");
            peer_ids.contains(&id)
        };
        if !is_tracked {
            return None;
        }

        self.find_available_peer(id)
    }

    /// Find a currently available peer without consulting legacy PeerSet
    /// membership. This is for a caller that has already selected an exact
    /// peer from its own availability snapshot, such as the acquisition
    /// coordinator. It must not be used for legacy PeerSet fan-out, whose
    /// tracked-membership semantics remain in [`Self::find_peer`].
    pub fn find_available_peer(&self, id: PeerId) -> Option<Arc<dyn Peer>> {
        let peers = self.peers.lock().expect("peer set lock");
        peers.iter().find(|peer| peer.id() == id).cloned()
    }

    /// Return tracked peers for parallel fan-out during state acquisition.
    pub fn get_peers(&self) -> Vec<Arc<dyn Peer>> {
        let peer_ids = self.peer_ids.lock().expect("peer ids lock").clone();
        let peers = self.peers.lock().expect("peer set lock");
        peers
            .iter()
            .filter(|peer| peer_ids.contains(&peer.id()))
            .cloned()
            .collect()
    }
}

impl PeerSet for SimplePeerSet {
    fn add_peers(
        &self,
        limit: usize,
        has_item: &mut dyn FnMut(&Arc<dyn Peer>) -> bool,
        on_peer_added: &mut dyn FnMut(&Arc<dyn Peer>),
    ) {
        // Mirror PeerSet's overlay walk without coupling its availability
        // snapshot to membership updates. This also keeps user callbacks out
        // of both locks, as InboundLedger schedules its follow-up work only
        // after addPeers has accepted a peer.
        let available = {
            let peers = self.peers.lock().expect("peer set lock");
            peers.iter().cloned().collect::<Vec<_>>()
        };
        let mut candidates = available
            .into_iter()
            .map(|peer| (peer.score(has_item(&peer)), peer))
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| right.0.cmp(&left.0));

        let added = {
            let mut peer_ids = self.peer_ids.lock().expect("peer ids lock");
            let mut added = Vec::new();
            for (_, peer) in candidates {
                if added.len() >= limit {
                    break;
                }
                if peer_ids.insert(peer.id()) {
                    added.push(peer);
                }
            }
            added
        };

        for peer in added {
            on_peer_added(&peer);
        }
    }

    fn send_request(&self, message: &ProtocolMessage, peer: Option<&Arc<dyn Peer>>) {
        let wire = crate::message::Message::new(message.clone(), None);
        if let Some(peer) = peer {
            peer.send(wire);
            return;
        }
        // Reference PeerSet::sendRequest(nullptr) sends only to `peers_`, the
        // tracked peer IDs. Sending to the availability deque lets an
        // untracked peer answer but then prevents `find_peer` from scheduling
        // the reply-driven follow-up that rippled relies on.
        let peer_ids = self.peer_ids.lock().expect("peer ids lock").clone();
        let peers = {
            let peers = self.peers.lock().expect("peer set lock");
            peers
                .iter()
                .filter(|peer| peer_ids.contains(&peer.id()))
                .cloned()
                .collect::<Vec<_>>()
        };
        for peer in peers {
            peer.send(wire.clone());
        }
    }

    fn peer_ids(&self) -> BTreeSet<PeerId> {
        self.peer_ids.lock().expect("peer ids lock").clone()
    }

    fn peer_count(&self) -> usize {
        self.peer_ids.lock().expect("peer ids lock").len()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Arc;

    use protocol::{KeyType, SecretKey, derive_public_key};

    use super::{PeerSet, SimplePeerSet};
    use crate::message::{ProtocolMessage, ProtocolPayload, TmPing};
    use crate::peer::Peer;
    use crate::peer_imp::PeerImp;

    fn peer(id: u32, seed: u8) -> Arc<dyn Peer> {
        let secret = SecretKey::from_bytes([seed; 32]);
        let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
        let peer: Arc<dyn Peer> = PeerImp::new(
            id,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 6000 + id as u16),
            public,
            format!("peer-{id}"),
        );
        peer
    }

    #[test]
    fn untargeted_request_reaches_only_tracked_peers() {
        let tracked = PeerImp::new(
            1,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 6001),
            derive_public_key(KeyType::Secp256k1, &SecretKey::from_bytes([1; 32]))
                .expect("public key"),
            "tracked",
        );
        let untracked = PeerImp::new(
            2,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 6002),
            derive_public_key(KeyType::Secp256k1, &SecretKey::from_bytes([2; 32]))
                .expect("public key"),
            "untracked",
        );
        let peer_set = SimplePeerSet::new(vec![
            Arc::clone(&tracked) as Arc<dyn Peer>,
            Arc::clone(&untracked) as Arc<dyn Peer>,
        ]);
        peer_set.add_peers(1, &mut |peer| peer.id() == 1, &mut |_| {});

        peer_set.send_request(
            &ProtocolMessage::new(ProtocolPayload::Ping(TmPing::default())),
            None,
        );

        assert_eq!(tracked.queued_messages().len(), 1);
        assert!(untracked.queued_messages().is_empty());
    }

    #[test]
    fn available_lookup_does_not_grant_legacy_membership() {
        let available = peer(7, 7);
        let peer_set = SimplePeerSet::new(vec![Arc::clone(&available)]);

        assert!(
            peer_set.find_peer(7).is_none(),
            "legacy lookup requires add_peers membership"
        );
        assert_eq!(
            peer_set
                .find_available_peer(7)
                .expect("available peer")
                .id(),
            7,
            "a coordinator-selected peer is deliverable without legacy membership"
        );
        assert!(peer_set.peer_ids().is_empty());
    }

    #[test]
    fn add_peers_sorts_by_score_and_skips_existing_ids() {
        let first = peer(1, 1);
        let second = peer(2, 2);
        let third = peer(3, 3);
        let peer_set = SimplePeerSet::new(vec![
            Arc::clone(&third),
            Arc::clone(&first),
            Arc::clone(&second),
        ]);
        let mut added = Vec::new();

        peer_set.add_peers(2, &mut |peer| peer.id() != 1, &mut |peer| {
            added.push(peer.id())
        });

        let added_ids = added.into_iter().collect::<BTreeSet<_>>();
        assert_eq!(added_ids, BTreeSet::from([2, 3]));

        let mut second_pass = Vec::new();
        peer_set.add_peers(3, &mut |_| true, &mut |peer| second_pass.push(peer.id()));
        assert_eq!(second_pass, vec![1]);
    }
}

pub struct SimplePeerSetBuilder {
    peers: Vec<Arc<dyn Peer>>,
}

impl SimplePeerSetBuilder {
    pub fn new(peers: Vec<Arc<dyn Peer>>) -> Self {
        Self { peers }
    }
}

impl PeerSetBuilder for SimplePeerSetBuilder {
    fn build(&self) -> Arc<dyn PeerSet> {
        Arc::new(SimplePeerSet::new(self.peers.iter().cloned()))
    }
}

#[derive(Default)]
pub struct DummyPeerSet;

impl PeerSet for DummyPeerSet {
    fn add_peers(
        &self,
        _limit: usize,
        _has_item: &mut dyn FnMut(&Arc<dyn Peer>) -> bool,
        _on_peer_added: &mut dyn FnMut(&Arc<dyn Peer>),
    ) {
    }

    fn send_request(&self, _message: &ProtocolMessage, _peer: Option<&Arc<dyn Peer>>) {}

    fn peer_ids(&self) -> BTreeSet<PeerId> {
        BTreeSet::new()
    }

    fn peer_count(&self) -> usize {
        0
    }
}

/// A PeerSetBuilder that dynamically queries the overlay for current
/// active peers at build() time. This ensures TransactionAcquire always
/// has a current peer list to send requests to, matching rippled's
/// InboundTransactions which calls app.overlay().getActivePeers()
/// each time it needs to send a request.
pub struct OverlayPeerSetBuilder {
    overlay: std::sync::Arc<crate::runtime::overlay_impl::OverlayImpl>,
}

impl OverlayPeerSetBuilder {
    pub fn new(overlay: std::sync::Arc<crate::runtime::overlay_impl::OverlayImpl>) -> Self {
        Self { overlay }
    }
}

impl PeerSetBuilder for OverlayPeerSetBuilder {
    fn build(&self) -> std::sync::Arc<dyn PeerSet> {
        use crate::Overlay;
        std::sync::Arc::new(SimplePeerSet::new(self.overlay.active_peers()))
    }
}
