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
        let peer_ids = self.peer_ids.lock().expect("peer ids lock");
        if !peer_ids.contains(&id) {
            return None;
        }
        let peers = self.peers.lock().expect("peer set lock");
        peers.iter().find(|p| p.id() == id).cloned()
    }

    /// Return all tracked peers for round-robin distribution.
    pub fn get_peers(&self) -> Vec<Arc<dyn Peer>> {
        let peer_ids = self.peer_ids.lock().expect("peer ids lock");
        let peers = self.peers.lock().expect("peer set lock");
        peers
            .iter()
            .filter(|p| peer_ids.contains(&p.id()))
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
        let peers = self.peers.lock().expect("peer set lock");
        let mut candidates = peers
            .iter()
            .map(|peer| (peer.score(has_item(peer)), Arc::clone(peer)))
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| right.0.cmp(&left.0));

        let mut peer_ids = self.peer_ids.lock().expect("peer ids lock");
        let mut added = 0usize;
        for (_, peer) in candidates {
            if added >= limit {
                break;
            }
            if !peer_ids.insert(peer.id()) {
                continue;
            }
            on_peer_added(&peer);
            added += 1;
        }
    }

    fn send_request(&self, message: &ProtocolMessage, peer: Option<&Arc<dyn Peer>>) {
        let wire = crate::message::Message::new(message.clone(), None);
        if let Some(peer) = peer {
            peer.send(wire);
            return;
        }
        // Broadcast to all tracked peers (reference iterates peers_ set)
        let peer_ids = self.peer_ids.lock().expect("peer ids lock");
        let peers = self.peers.lock().expect("peer set lock");
        for p in peers.iter() {
            if peer_ids.contains(&p.id()) {
                p.send(wire.clone());
            }
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
