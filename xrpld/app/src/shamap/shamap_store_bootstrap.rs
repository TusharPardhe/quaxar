use crate::shamap::shamap_store_config::node_db_section;
use crate::{
    SHAMapStore, SHAMapStoreBackendBundle, SHAMapStoreNodeStore, SHAMapStoreSavedState,
    SHAMapStoreSavedStateDb, apply_rocksdb_online_delete_defaults, make_shamap_store_backend,
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
    let node_db = apply_rocksdb_online_delete_defaults(
        node_db_section(config)?,
        hash_node_db_cache_mb,
        node_size,
    );
    let mut store = SHAMapStore::from_config(config, standalone, ledger_history, 0)?;

    let saved_state = if store.delete_interval() != 0 {
        let state_db = SHAMapStoreSavedStateDb::open(config, "state")?;
        let saved_state = state_db.get_state()?;
        let SHAMapStoreBackendBundle {
            store: node_store,
            fd_required,
            saved_state: next_state,
        } = make_shamap_store_backend(
            manager,
            scheduler,
            read_threads,
            &node_db,
            store.delete_interval(),
            &saved_state,
            burst_size,
            journal,
        )?;
        if next_state != saved_state {
            state_db.set_state(&next_state)?;
        }
        store.set_saved_state(next_state.clone());
        store.set_fd_required(fd_required);
        return Ok(SHAMapStoreBootstrap {
            store,
            node_store,
            state_db: Some(state_db),
            effective_node_db_config: node_db,
        });
    } else {
        SHAMapStoreSavedState::default()
    };

    let SHAMapStoreBackendBundle {
        store: node_store,
        fd_required,
        ..
    } = make_shamap_store_backend(
        manager,
        scheduler,
        read_threads,
        &node_db,
        0,
        &saved_state,
        burst_size,
        journal,
    )?;
    store.set_saved_state(saved_state);
    store.set_fd_required(fd_required);
    Ok(SHAMapStoreBootstrap {
        store,
        node_store,
        state_db: None,
        effective_node_db_config: node_db,
    })
}
