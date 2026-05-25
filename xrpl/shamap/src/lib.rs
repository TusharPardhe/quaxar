//! `xrpl/shamap` crate surface.
//!
//! This crate provides a full-parity SHAMap stack:
//! - the shared `SHAMapTreeNode` ownership boundary,
//! - inner-node child and hash storage roles,
//! - `TreeNodeCache` over intrusive pointers,
//! - deterministic leaf hash recomputation and child canonicalization,
//! - comprehensive wire/prefix byte codecs for all node kinds.

pub mod nodes;
pub mod operations;
pub mod owners;
pub mod traverse;

pub use nodes::*;
pub use operations::*;
pub use owners::*;
pub use traverse::*;
