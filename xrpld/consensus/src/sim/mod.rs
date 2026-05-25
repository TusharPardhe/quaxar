//! Consensus Simulation Framework (CSF) — full port of reference `test/csf/`.
//!
//! Provides a discrete-event simulation of multiple consensus peers connected
//! via a network graph with configurable delays and trust relationships.
//!
//! ## Module Structure
//! - `types` — SimTime, Tx, TxSet, Ledger, LedgerOracle, Proposal
//! - `scheduler` — Discrete event scheduler with timers
//! - `graph` — Digraph and BasicNetwork
//! - `trust` — TrustGraph and PeerGroup
//! - `peer` — Full consensus peer (Adaptor implementation)
//! - `collectors` — Statistics collection and event tracking
//! - `sim` — Top-level Sim orchestrator

pub mod collectors;
pub mod graph;
pub mod orchestrator;
pub mod peer;
pub mod scheduler;
pub mod trust;
pub mod types;
