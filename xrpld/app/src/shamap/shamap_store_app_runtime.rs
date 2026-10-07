//! Runtime trait surface the SHAMap store component drives.
//!
//! The rotating application runtime that once lived here was removed with the
//! rotating node store. What remains are the small runtime traits the
//! component and its collaborators still implement: the fjall pruned store is
//! driven through the no-op defaults on these traits plus the pruned driver.

use crate::shamap::shamap_store_component::SHAMapStoreComponentRuntime;
use crate::shamap::shamap_store_copy::SHAMapStoreCopyDisposition;
use basics::base_uint::Uint256;
use basics::blob::Blob;
use basics::memory::intrusive_pointer::SharedIntrusive;
use ledger::Ledger;
use shamap::traversal::TraversalError;
use std::sync::Arc;

pub trait SHAMapStoreLedgerRuntime: Send + Sync {
    fn clear_prior_ledgers(&self, last_rotated: u32);
    fn clear_online_delete_caches(&self, validated_seq: u32);

    /// Count missing entries from the synchronized complete-ledger range.
    fn missing_from_complete_ledger_range(&self, _first: u32, _last: u32) -> usize {
        0
    }

    fn has_complete_ledger(&self, _seq: u32) -> bool {
        true
    }
}

pub trait SHAMapStoreNodeFamilyCacheRuntime: Send + Sync {
    fn tree_node_cache_keys(&self) -> Vec<Uint256>;
    fn clear_full_below_cache(&self);
    fn visit_state_map_nodes(
        &self,
        ledger: &Ledger,
        visit: &mut dyn FnMut(&SharedIntrusive<shamap::tree_node::SHAMapTreeNode>) -> bool,
    ) -> Result<(), TraversalError>;
}

pub trait SHAMapStoreTransactionCacheRuntime: Send + Sync {
    fn cache_keys(&self) -> Vec<Uint256>;
}

pub trait SHAMapStoreNodeStoreRuntime: Send + Sync {
    fn fetch_node_object(&self, hash: &Uint256, ledger_seq: u32) -> bool;

    fn copy_to_writable_batch(&self, hashes: &[Uint256]) -> Result<usize, String> {
        Ok(hashes
            .iter()
            .filter(|hash| self.fetch_node_object(hash, 0))
            .count())
    }

    fn copy_to_writable_batch_detailed(
        &self,
        hashes: &[Uint256],
    ) -> Result<(usize, Vec<Uint256>), String> {
        self.copy_to_writable_batch(hashes)
            .map(|copied| (copied, Vec::new()))
    }

    fn store_account_nodes(&self, _nodes: Vec<(Uint256, Blob)>) -> Result<(), String> {
        Err("NodeStore runtime does not support resident-node rescue".to_owned())
    }
}

/// RAII token returned by a component runtime while rotating. Rotation is gone,
/// so the only implementation is the component's no-op window, but the trait is
/// retained as the component runtime's associated window type.
pub trait SHAMapStoreRotationWindow: Send {}

pub trait SHAMapStoreCopyRuntime: Send + Sync {
    fn copy_validated_ledger(
        &self,
        _validated_ledger: Arc<Ledger>,
        _node_family: &dyn SHAMapStoreNodeFamilyCacheRuntime,
        _node_store: &dyn SHAMapStoreNodeStoreRuntime,
        _runtime: &mut dyn SHAMapStoreComponentRuntime,
        _health_policy: crate::SHAMapStoreHealthPolicy,
    ) -> Result<SHAMapStoreCopyDisposition, String> {
        Ok(SHAMapStoreCopyDisposition::Completed { node_count: 0 })
    }
}

#[derive(Debug, Default)]
pub struct NullSHAMapStoreCopyRuntime;

impl SHAMapStoreCopyRuntime for NullSHAMapStoreCopyRuntime {}
