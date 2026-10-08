use crate::SHAMapStoreSavedState;
use basics::basic_config::Section;
use nodestore::{Backend, Database, Manager, NodeStoreJournal, Scheduler};
use std::sync::Arc;

/// The application node store. Only the single-database shape remains: the
/// fjall pruned store (which prunes continuously) and the in-memory/null
/// stores all present as a single `Database`. The historical rotating
/// (rotating copy-forward) variant has been removed.
#[derive(Clone)]
pub enum SHAMapStoreNodeStore {
    Single(Arc<dyn Database>),
}

impl SHAMapStoreNodeStore {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Single(_) => "single",
        }
    }

    pub fn fd_required(&self) -> i32 {
        match self {
            Self::Single(database) => database.fd_required(),
        }
    }

    /// Stable nonzero NodeStore generation observed by local-read admission.
    /// Callers include it in `ReadKey` and durable callback identities.
    pub fn store_generation(&self) -> u64 {
        match self {
            Self::Single(database) => database.store_generation(),
        }
    }

    pub fn schedule_write(&self, write: nodestore::ScheduledWrite) {
        match self {
            Self::Single(database) => database.schedule_write(write),
        }
    }

    pub fn export_backend(&self) -> Option<Arc<dyn Backend>> {
        match self {
            Self::Single(database) => database.export_backend(),
        }
    }
}

pub struct SHAMapStoreBackendBundle {
    pub store: SHAMapStoreNodeStore,
    pub fd_required: i32,
    pub saved_state: SHAMapStoreSavedState,
}

impl SHAMapStoreBackendBundle {
    pub fn node_store_kind(&self) -> &'static str {
        self.store.kind()
    }
}

pub fn make_shamap_store_backend(
    manager: &dyn Manager,
    scheduler: Arc<dyn Scheduler>,
    read_threads: i32,
    node_db: &Section,
    state: &SHAMapStoreSavedState,
    burst_size: usize,
    journal: Arc<dyn NodeStoreJournal>,
) -> Result<SHAMapStoreBackendBundle, String> {
    // The node store is always a single database. Online deletion is provided
    // by the fjall pruned store (driven separately), not by backend rotation.
    let database = manager.make_database(
        burst_size,
        scheduler,
        read_threads,
        node_db,
        Arc::clone(&journal),
    )?;
    let fd_required = database.fd_required();
    Ok(SHAMapStoreBackendBundle {
        store: SHAMapStoreNodeStore::Single(database),
        fd_required,
        saved_state: state.clone(),
    })
}
