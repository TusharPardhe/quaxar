use crate::shamap::shamap_store_config::node_db_section;
use crate::{
    SHAMapStore, SHAMapStoreBackendBundle, SHAMapStoreNodeStore, SHAMapStoreSavedState,
    SHAMapStoreSavedStateDb, make_shamap_store_backend,
};
use basics::basic_config::{BasicConfig, Section};
use nodestore::{Manager, NodeStoreJournal, Scheduler};
use std::sync::Arc;

pub struct SHAMapStoreBootstrap {
    pub store: SHAMapStore,
    pub node_store: SHAMapStoreNodeStore,
    pub state_db: Option<SHAMapStoreSavedStateDb>,
    pub effective_node_db_config: Section,
}

impl SHAMapStoreBootstrap {
    pub fn attach_node_store(
        &self,
        root: &mut crate::ApplicationRoot,
    ) -> Option<SHAMapStoreNodeStore> {
        root.attach_node_store(Some(self.node_store.clone()))
    }

    pub fn node_store_kind(&self) -> &'static str {
        self.node_store.kind()
    }
}

#[allow(clippy::too_many_arguments)]
pub fn bootstrap_shamap_store(
    config: &BasicConfig,
    standalone: bool,
    ledger_history: u32,
    read_threads: i32,
    burst_size: usize,
    hash_node_db_cache_mb: usize,
    node_size: u32,
    manager: &dyn Manager,
    scheduler: Arc<dyn Scheduler>,
    journal: Arc<dyn NodeStoreJournal>,
) -> Result<SHAMapStoreBootstrap, String> {
    // RocksDB tuning defaults were dropped with the RocksDB backend; the
    // node_db section is used as configured.
    let _ = (hash_node_db_cache_mb, node_size);
    let mut node_db = node_db_section(config)?.clone();
    let configured_node_size = config
        .exists("node_size")
        .then(|| config.section("node_size").values().first().cloned())
        .flatten();
    let profile = crate::NodeSizeResourceProfile::for_node_size(configured_node_size.as_deref());
    if !node_db.exists("node_object_cache_target_nodes")
        && !node_db.exists("cache_size")
        && !node_db.exists("cache_capacity_mb")
        && !node_db.exists("node_object_cache_capacity_bytes")
    {
        node_db.set(
            "node_object_cache_target_nodes",
            profile.tree_cache_size.to_string(),
        );
    }
    if !node_db.exists("cache_idle_seconds") && !node_db.exists("cache_age") {
        // rippled injects TreeCacheAge into NodeStore, whose cache interprets
        // the value in minutes (decoded TreeNodeCache uses seconds).
        node_db.set(
            "cache_idle_seconds",
            profile
                .tree_cache_age_seconds
                .saturating_mul(60)
                .to_string(),
        );
    }
    if !node_db.exists("cache_ttl_seconds") {
        node_db.set("cache_ttl_seconds", "0");
    }
    let mut store = SHAMapStore::from_config(config, standalone, ledger_history, 0)?;

    // online_delete no longer selects a rotating store: the node store is always
    // a single database, and the fjall path prunes continuously through the
    // PrunedDriver using online_delete as its retention window. The interval is
    // cleared here so the SHAMapStore never schedules the removed rotation; the
    // validated value stays in the node_db section for the driver.
    store.set_delete_interval(0);
    // The saved-state DB persists the advisory can_delete boundary across
    // restarts, so it is still opened when advisory deletion is enabled.
    let state_db = if store.advisory_delete() {
        Some(SHAMapStoreSavedStateDb::open(config, "state")?)
    } else {
        None
    };

    let SHAMapStoreBackendBundle {
        store: node_store,
        fd_required,
        saved_state,
    } = make_shamap_store_backend(
        manager,
        scheduler,
        read_threads,
        &node_db,
        &SHAMapStoreSavedState::default(),
        burst_size,
        journal,
    )?;
    store.set_saved_state(saved_state);
    store.set_fd_required(fd_required);
    Ok(SHAMapStoreBootstrap {
        store,
        node_store,
        state_db,
        effective_node_db_config: node_db,
    })
}
