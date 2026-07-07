//! `consensus`: the generic, adaptor-parameterized XRP Ledger consensus
//! algorithm, ported from rippled's `xrpld/consensus` C++ sources.
//!
//! This crate contains only the pure algorithm and its supporting data
//! structures. It has no knowledge of the ledger, overlay, or job queue —
//! those are wired in by the `app` crate through the `ConsensusAdaptor` and
//! `RclConsensusAdapter` traits.

pub mod algorithm;
pub mod model;
pub mod rcl_support;
