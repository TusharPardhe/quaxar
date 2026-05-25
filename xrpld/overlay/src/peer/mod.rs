#![allow(clippy::module_inception)]
pub mod peer;
pub mod peer_imp;
pub mod peer_set;
pub mod predicates;
pub mod slot;
pub mod squelch;
pub(crate) mod status_change;

pub use peer::{Peer, PeerId, ProtocolFeature};
pub use peer_imp::PeerImp;
pub use peer_set::{DummyPeerSet, PeerSet, PeerSetBuilder, SimplePeerSet, SimplePeerSetBuilder};
pub use predicates::{
    MatchPeer, PeerInCluster, PeerInSet, SendAlways, SendIf, SendIfNot, send_if, send_if_not,
};
pub use slot::{
    Clock, ManualClock, PeerState, Slot, SlotPeerSnapshot, SlotState, Slots, SquelchHandler,
    SystemClock,
};
pub use squelch::Squelch;
