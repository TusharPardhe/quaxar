//! Pruned node store: the engine-agnostic index that keeps only the nodes a
//! sliding window of validated ledgers needs, using a stale-node index plus
//! reference counts (the Jellyfish Merkle Tree / Aptos technique).
//!
//! This module currently provides [`model`], the reference oracle that later
//! stages test the real [`IndexWriter`](index) against. The index writer,
//! prune loop, orphan sweep, reconciler and verifier land in subsequent
//! stages; see `docs/design/2026-10-07-fjall-pruned-nodestore.md`.

pub mod model;

pub use model::{LedgerSnapshot, ModelStore};
