//! Overlay send predicates mirrored from `overlay/predicates.h`.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::message::Message;
use crate::peer::{Peer, PeerId};

#[derive(Clone)]
pub struct SendAlways {
    msg: Message,
}

impl SendAlways {
    pub fn new(msg: Message) -> Self {
        Self { msg }
    }

    pub fn apply(&self, peer: &Arc<dyn Peer>) {
        peer.send(self.msg.clone());
    }
}

#[derive(Clone)]
pub struct SendIf<P> {
    msg: Message,
    predicate: P,
}

impl<P> SendIf<P>
where
    P: Fn(&Arc<dyn Peer>) -> bool + Clone,
{
    pub fn apply(&self, peer: &Arc<dyn Peer>) {
        if (self.predicate)(peer) {
            peer.send(self.msg.clone());
        }
    }
}

pub fn send_if<P>(msg: Message, predicate: P) -> SendIf<P>
where
    P: Fn(&Arc<dyn Peer>) -> bool + Clone,
{
    SendIf { msg, predicate }
}

#[derive(Clone)]
pub struct SendIfNot<P> {
    msg: Message,
    predicate: P,
}

impl<P> SendIfNot<P>
where
    P: Fn(&Arc<dyn Peer>) -> bool + Clone,
{
    pub fn apply(&self, peer: &Arc<dyn Peer>) {
        if !(self.predicate)(peer) {
            peer.send(self.msg.clone());
        }
    }
}

pub fn send_if_not<P>(msg: Message, predicate: P) -> SendIfNot<P>
where
    P: Fn(&Arc<dyn Peer>) -> bool + Clone,
{
    SendIfNot { msg, predicate }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchPeer {
    peer_id: Option<PeerId>,
}

impl MatchPeer {
    pub fn new(peer_id: Option<PeerId>) -> Self {
        Self { peer_id }
    }

    pub fn matches(&self, peer: &Arc<dyn Peer>) -> bool {
        self.peer_id.is_some_and(|peer_id| peer.id() == peer_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerInCluster {
    skip_peer: MatchPeer,
}

impl PeerInCluster {
    pub fn new(skip_peer: Option<PeerId>) -> Self {
        Self {
            skip_peer: MatchPeer::new(skip_peer),
        }
    }

    pub fn matches(&self, peer: &Arc<dyn Peer>) -> bool {
        !self.skip_peer.matches(peer) && peer.cluster()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerInSet {
    peer_set: BTreeSet<PeerId>,
}

impl PeerInSet {
    pub fn new(peer_set: BTreeSet<PeerId>) -> Self {
        Self { peer_set }
    }

    pub fn matches(&self, peer: &Arc<dyn Peer>) -> bool {
        self.peer_set.contains(&peer.id())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::collections::HashSet;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Arc;

    use basics::base_uint::Uint256;
    use protocol::{JsonValue, KeyType, PublicKey, SecretKey, derive_public_key};
    use resource::Charge;

    use super::{MatchPeer, PeerInCluster, PeerInSet};
    use crate::message::Message;
    use crate::peer::{Peer, ProtocolFeature};

    struct TestPeer {
        id: u32,
        cluster: bool,
    }

    impl Peer for TestPeer {
        fn send(&self, _message: Message) {}
        fn remote_address(&self) -> SocketAddr {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51235)
        }
        fn send_tx_queue(&self) {}
        fn add_tx_queue(&self, _hash: Uint256) {}
        fn remove_tx_queue(&self, _hash: Uint256) {}
        fn charge(&self, _fee: Charge, _context: String) {}
        fn id(&self) -> u32 {
            self.id
        }
        fn cluster(&self) -> bool {
            self.cluster
        }
        fn is_high_latency(&self) -> bool {
            false
        }
        fn score(&self, _clustered: bool) -> i32 {
            0
        }
        fn node_public(&self) -> PublicKey {
            let secret = SecretKey::from_bytes([self.id as u8; 32]);
            derive_public_key(KeyType::Secp256k1, &secret).expect("public key")
        }
        fn json(&self) -> JsonValue {
            JsonValue::Null
        }
        fn supports_feature(&self, _feature: ProtocolFeature) -> bool {
            false
        }
        fn publisher_list_sequence(&self, _publisher: PublicKey) -> Option<usize> {
            None
        }
        fn set_publisher_list_sequence(&self, _publisher: PublicKey, _sequence: usize) {}
        fn fingerprint(&self) -> String {
            String::new()
        }
        fn closed_ledger_hash(&self) -> Uint256 {
            Uint256::zero()
        }
        fn previous_ledger_hash(&self) -> Uint256 {
            Uint256::zero()
        }
        fn has_ledger(&self, _hash: Uint256, _sequence: u32) -> bool {
            false
        }
        fn ledger_range(&self) -> (u32, u32) {
            (0, 0)
        }
        fn has_tx_set(&self, _hash: Uint256) -> bool {
            false
        }
        fn cycle_status(&self) {}
        fn has_range(&self, _min_sequence: u32, _max_sequence: u32) -> bool {
            false
        }
        fn compression_enabled(&self) -> bool {
            false
        }
        fn tx_reduce_relay_enabled(&self) -> bool {
            false
        }
        fn features(&self) -> HashSet<ProtocolFeature> {
            HashSet::new()
        }
    }

    #[test]
    fn predicates_match_cpp_shapes() {
        let cluster_peer: Arc<dyn Peer> = Arc::new(TestPeer {
            id: 7,
            cluster: true,
        });
        let outsider: Arc<dyn Peer> = Arc::new(TestPeer {
            id: 9,
            cluster: false,
        });

        assert!(MatchPeer::new(Some(7)).matches(&cluster_peer));
        assert!(!MatchPeer::new(Some(7)).matches(&outsider));
        assert!(PeerInCluster::new(None).matches(&cluster_peer));
        assert!(!PeerInCluster::new(Some(7)).matches(&cluster_peer));
        assert!(PeerInSet::new(BTreeSet::from([7])).matches(&cluster_peer));
        assert!(!PeerInSet::new(BTreeSet::from([7])).matches(&outsider));
    }
}
