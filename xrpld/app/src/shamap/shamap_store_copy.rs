use crate::shamap::shamap_store_component::SHAMapStoreComponentRuntime;
use crate::shamap::shamap_store_health::{
    SHAMapStoreHealthPolicy, SHAMapStoreHealthStatus, wait_for_health,
};
use crate::{
    SHAMapStoreCopyRuntime, SHAMapStoreNodeFamilyCacheRuntime, SHAMapStoreNodeStoreRuntime,
};
use basics::base_uint::Uint256;
use ledger::Ledger;
use shamap::traversal::TraversalError;
use std::sync::Arc;

pub const SHAMAP_STORE_COPY_CHECK_HEALTH_INTERVAL: u64 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SHAMapStoreCopyDisposition {
    Completed { node_count: u64 },
    Stopped { node_count: u64 },
    MissingNode { hash: Uint256, node_count: u64 },
}

#[derive(Debug, Default)]
pub struct ValidatedLedgerCopyRuntime;

impl SHAMapStoreCopyRuntime for ValidatedLedgerCopyRuntime {
    fn copy_validated_ledger(
        &self,
        validated_ledger: Arc<Ledger>,
        node_family: &dyn SHAMapStoreNodeFamilyCacheRuntime,
        node_store: &dyn SHAMapStoreNodeStoreRuntime,
        runtime: &mut dyn SHAMapStoreComponentRuntime,
        health_policy: SHAMapStoreHealthPolicy,
    ) -> Result<SHAMapStoreCopyDisposition, String> {
        copy_validated_state_map(
            validated_ledger,
            node_family,
            node_store,
            runtime,
            health_policy,
        )
    }
}

pub fn copy_validated_state_map(
    validated_ledger: Arc<Ledger>,
    node_family: &dyn SHAMapStoreNodeFamilyCacheRuntime,
    node_store: &dyn SHAMapStoreNodeStoreRuntime,
    runtime: &mut dyn SHAMapStoreComponentRuntime,
    health_policy: SHAMapStoreHealthPolicy,
) -> Result<SHAMapStoreCopyDisposition, String> {
    let mut node_count = 0u64;
    let mut stopped = false;
    let visit_result = node_family.visit_state_map_hashes(validated_ledger.as_ref(), &mut |hash| {
        let _ = node_store.fetch_node_object(&hash, 0);
        node_count += 1;

        if !node_count.is_multiple_of(SHAMAP_STORE_COPY_CHECK_HEALTH_INTERVAL) {
            return true;
        }

        let keep_going = wait_for_health(&health_policy, runtime, |runtime, duration| {
            runtime.sleep(duration);
        }) != SHAMapStoreHealthStatus::Stopping;
        stopped = !keep_going;
        keep_going
    });

    match visit_result {
        Ok(()) if stopped => Ok(SHAMapStoreCopyDisposition::Stopped { node_count }),
        Ok(()) => Ok(SHAMapStoreCopyDisposition::Completed { node_count }),
        Err(TraversalError::MissingNode(hash)) => Ok(SHAMapStoreCopyDisposition::MissingNode {
            hash: *hash.as_uint256(),
            node_count,
        }),
        Err(e) => Err(format!("Traversal error: {:?}", e)),
    }
}
