//! Peer abstraction and feature flags.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use basics::base_uint::Uint256;
use protocol::{JsonValue, PublicKey};
use resource::Charge;

use crate::message::Message;

pub type PeerId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolFeature {
    ValidatorListPropagation,
    ValidatorList2Propagation,
    LedgerReplay,
}

pub trait Peer: Send + Sync {
    fn send(&self, message: Message);
    fn remote_address(&self) -> SocketAddr;
    fn send_tx_queue(&self);
    fn add_tx_queue(&self, hash: Uint256);
    fn remove_tx_queue(&self, hash: Uint256);
    fn charge(&self, fee: Charge, context: String);
    fn id(&self) -> PeerId;
    fn cluster(&self) -> bool;
    fn is_high_latency(&self) -> bool;
    fn score(&self, clustered: bool) -> i32;
    fn node_public(&self) -> PublicKey;
    fn json(&self) -> JsonValue;
    fn supports_feature(&self, feature: ProtocolFeature) -> bool;
    fn publisher_list_sequence(&self, publisher: PublicKey) -> Option<usize>;
    fn set_publisher_list_sequence(&self, publisher: PublicKey, sequence: usize);
    fn fingerprint(&self) -> String;
    fn closed_ledger_hash(&self) -> Uint256;
    fn previous_ledger_hash(&self) -> Uint256;
    fn has_ledger(&self, hash: Uint256, sequence: u32) -> bool;
    fn ledger_range(&self) -> (u32, u32);
    fn has_tx_set(&self, hash: Uint256) -> bool;
    fn cycle_status(&self);
    fn has_range(&self, min_sequence: u32, max_sequence: u32) -> bool;
    fn compression_enabled(&self) -> bool;
    fn tx_reduce_relay_enabled(&self) -> bool;
    fn features(&self) -> HashSet<ProtocolFeature>;
    fn should_filter_recent_endpoint(
        &self,
        _endpoint: SocketAddr,
        _hops: u32,
        _now: Instant,
        _ttl: Duration,
    ) -> bool {
        false
    }
    fn remember_recent_endpoint(
        &self,
        _endpoint: SocketAddr,
        _hops: u32,
        _now: Instant,
        _ttl: Duration,
    ) {
    }
}
