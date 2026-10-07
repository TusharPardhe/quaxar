use crate::SHAMapStoreSavedState;
use basics::basic_config::Section;
use nodestore::{Backend, Database, Manager, NodeStoreJournal, Scheduler};
use std::sync::Arc;

/// The application node store. Only the single-database shape remains: the
/// fjall pruned store (which prunes continuously) and the in-memory/null
/// stores all present as a single `Database`. The historical rotating
/// (NuDB/RocksDB copy-forward) variant has been removed.
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
    delete_interval: u32,
    state: &SHAMapStoreSavedState,
    burst_size: usize,
    journal: Arc<dyn NodeStoreJournal>,
) -> Result<SHAMapStoreBackendBundle, String> {
    // The rotating node store has been removed. Online deletion is now provided
    // by the fjall pruned store, which is a single database (delete_interval is
    // zero at this layer; pruning is driven separately). A nonzero rotating
    // delete_interval reaching here means an unsupported backend slipped past
    // config validation.
    if delete_interval != 0 {
        return Err(
            "rotating online_delete is no longer supported; use [node_db] type = fjall".to_owned(),
        );
    }
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

/// Open a single backend at a resolved path. Retained for the (now unused)
/// configured backend factory; the rotating store that once consumed pairs of
/// these has been removed.
pub fn make_shamap_store_rotating_backend(
    manager: &dyn Manager,
    node_db: &Section,
    burst_size: usize,
    scheduler: Arc<dyn Scheduler>,
    journal: Arc<dyn NodeStoreJournal>,
    path: Option<&str>,
) -> Result<Box<dyn Backend>, String> {
    let mut section = node_db.clone();
    if let Some(path) = path {
        section.set("path", path);
    }
    let backend = manager.make_backend(&section, burst_size, scheduler, journal)?;
    backend.open(true)?;
    Ok(backend)
}
