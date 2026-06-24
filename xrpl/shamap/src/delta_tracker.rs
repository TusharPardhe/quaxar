//! Incremental delta hash tracker for SHAMap mutations.
//!
//! Records modified leaf keys so that rehashing can be scoped to only
//! the dirty paths rather than recomputing the full tree.

use basics::base_uint::Uint256;

/// Tracks which SHAMap leaf keys have been modified during a mutation batch.
#[derive(Debug, Clone, Default)]
pub struct DeltaTracker {
    dirty: Vec<Uint256>,
}

impl DeltaTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a leaf key that was inserted, updated, or removed.
    pub fn track_modification(&mut self, key: Uint256) {
        self.dirty.push(key);
    }

    /// Return all dirty leaf keys recorded since the last clear.
    pub fn dirty_paths(&self) -> &[Uint256] {
        &self.dirty
    }

    /// Reset the tracker for a new mutation batch.
    pub fn clear(&mut self) {
        self.dirty.clear();
    }
}

/// Shadow validation: recompute full hash and compare against incremental.
/// Only compiled when the `shadow-hash` feature is active.
#[cfg(feature = "shadow-hash")]
pub fn shadow_validate_hash(incremental: &Uint256, full: &Uint256) -> bool {
    incremental == full
}
