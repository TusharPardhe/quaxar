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
pub const SHAMAP_STORE_COPY_BATCH_SIZE: usize = 64;

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
    let mut copy_error = None;
    let mut pending = Vec::with_capacity(SHAMAP_STORE_COPY_BATCH_SIZE);
    let visit_result = node_family.visit_state_map_hashes(validated_ledger.as_ref(), &mut |hash| {
        pending.push(hash);
        node_count += 1;

        if pending.len() == SHAMAP_STORE_COPY_BATCH_SIZE
            && let Err(error) = node_store.copy_to_writable_batch(&pending)
        {
            copy_error = Some(error);
            return false;
        }
        if pending.len() == SHAMAP_STORE_COPY_BATCH_SIZE {
            pending.clear();
        }

        if !node_count.is_multiple_of(SHAMAP_STORE_COPY_CHECK_HEALTH_INTERVAL) {
            return true;
        }

        let keep_going = wait_for_health(&health_policy, runtime, |runtime, duration| {
            runtime.sleep(duration);
        }) != SHAMapStoreHealthStatus::Stopping;
        stopped = !keep_going;
        keep_going
    });

    if let Some(error) = copy_error {
        return Err(error);
    }
    if !pending.is_empty() {
        node_store.copy_to_writable_batch(&pending)?;
    }

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
