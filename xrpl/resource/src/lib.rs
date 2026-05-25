//! Rust parity crate for the reference `Resource::Manager` surface.
//!
//! The reference implementation keeps a ref-counted consumer table, decaying local
//! balances, gossip import/export, warning and disconnect thresholds, and a
//! lightweight reporting surface. This crate ports that behavior with explicit
//! tests so the public flows stay aligned with the reference reference.

pub mod api;
pub mod core;

pub use api::*;
pub use core::*;

pub use charge::Charge;
pub use consumer::Consumer;
pub use fees::{
    DROP_THRESHOLD, FEE_DROP, FEE_EXCEPTION_RPC, FEE_HEAVY_BURDEN_PEER, FEE_HEAVY_BURDEN_RPC,
    FEE_INVALID_DATA, FEE_INVALID_SIGNATURE, FEE_LOG_AS_DEBUG, FEE_LOG_AS_INFO, FEE_LOG_AS_WARN,
    FEE_MALFORMED_REQUEST, FEE_MALFORMED_RPC, FEE_MEDIUM_BURDEN_RPC, FEE_MODERATE_BURDEN_PEER,
    FEE_REFERENCE_RPC, FEE_REQUEST_NO_REPLY, FEE_TRIVIAL_PEER, FEE_USELESS_DATA, FEE_WARNING,
    MINIMUM_GOSSIP_BALANCE, WARNING_THRESHOLD,
};
pub use gossip::{Gossip, GossipItem, PublicKey};
pub use logic::{
    DECAY_WINDOW_SECONDS, Disposition, JournalLevel, Kind, NullCollector, NullJournal, NullMeter,
    ResourceClock, ResourceCollector, ResourceJournal, ResourceMeter, SECONDS_UNTIL_EXPIRATION,
};
pub use manager::{ResourceManager, make_manager};
pub use types::GOSSIP_EXPIRATION_SECONDS;
pub use types::{
    DROP_THRESHOLD as dropThreshold, FEE_DROP as feeDrop, FEE_EXCEPTION_RPC as feeExceptionRPC,
    FEE_HEAVY_BURDEN_PEER as feeHeavyBurdenPeer, FEE_HEAVY_BURDEN_RPC as feeHeavyBurdenRPC,
    FEE_INVALID_DATA as feeInvalidData, FEE_INVALID_SIGNATURE as feeInvalidSignature,
    FEE_LOG_AS_DEBUG as feeLogAsDebug, FEE_LOG_AS_INFO as feeLogAsInfo,
    FEE_LOG_AS_WARN as feeLogAsWarn, FEE_MALFORMED_REQUEST as feeMalformedRequest,
    FEE_MALFORMED_RPC as feeMalformedRPC, FEE_MEDIUM_BURDEN_RPC as feeMediumBurdenRPC,
    FEE_MODERATE_BURDEN_PEER as feeModerateBurdenPeer, FEE_REFERENCE_RPC as feeReferenceRPC,
    FEE_REQUEST_NO_REPLY as feeRequestNoReply, FEE_TRIVIAL_PEER as feeTrivialPeer,
    FEE_USELESS_DATA as feeUselessData, FEE_WARNING as feeWarning,
    WARNING_THRESHOLD as warningThreshold,
};
