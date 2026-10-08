use crate::runtime::main_runtime::ManagedComponent;
use crate::{
    SHAMapStore, SHAMapStoreHealthRuntime, SHAMapStoreRuntime, SHAMapStoreSavedState,
    SHAMapStoreSavedStateDb,
};
use ledger::Ledger;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

/// The runtime the SHAMap store component drives. The rotating online-delete
/// worker was removed with NuDB, so this is now a marker over the two runtime
/// surfaces the store lifecycle (`start`/`stop`) still needs. The fjall pruned
/// path advances through `on_ledger_closed` -> `PrunedDriver`, not a worker.
pub trait SHAMapStoreComponentRuntime:
    SHAMapStoreRuntime + SHAMapStoreHealthRuntime + Send + 'static
{
}

struct SHAMapStoreComponentInner {
    store: Mutex<SHAMapStore>,
    runtime: Mutex<Box<dyn SHAMapStoreComponentRuntime>>,
    state_db: Option<Arc<SHAMapStoreSavedStateDb>>,
    /// Mirrors rippled's atomic `canDelete_`: observed at the next advisory
    /// boundary without taking the producer store mutex.
    can_delete: AtomicU32,
    /// Present only for the fjall pruned-store path. When set, on_ledger_closed
    /// drives it (claim + prune).
    pruned: Option<Arc<crate::shamap::pruned_driver::PrunedDriver>>,
}

pub struct SHAMapStoreComponent {
    inner: Arc<SHAMapStoreComponentInner>,
}

impl std::fmt::Debug for SHAMapStoreComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SHAMapStoreComponent")
            .field(
                "store",
                &self
                    .inner
                    .store
                    .lock()
                    .expect("shamap store mutex must not be poisoned"),
            )
            .field("has_state_db", &self.inner.state_db.is_some())
            .finish()
    }
}

impl SHAMapStoreComponent {
    pub fn new(
        store: SHAMapStore,
        runtime: Box<dyn SHAMapStoreComponentRuntime>,
        state_db: Option<SHAMapStoreSavedStateDb>,
    ) -> Self {
        let can_delete = store.get_can_delete();
        Self {
            inner: Arc::new(SHAMapStoreComponentInner {
                store: Mutex::new(store),
                runtime: Mutex::new(runtime),
                state_db: state_db.map(Arc::new),
                can_delete: AtomicU32::new(can_delete),
                pruned: None,
            }),
        }
    }

    /// Attach a pruned-store driver (the fjall path). When present, the
    /// component drives claim + prune on each validated ledger and leaves the
    /// rotation worker idle. Builder form so the existing constructors and
    /// their call sites are unchanged.
    pub fn with_pruned_driver(
        mut self,
        pruned: Arc<crate::shamap::pruned_driver::PrunedDriver>,
    ) -> Self {
        // The inner is freshly created in `new` and not yet shared, so a
        // get_mut here is guaranteed to succeed.
        Arc::get_mut(&mut self.inner)
            .expect("component inner is unique before the worker starts")
            .pruned = Some(pruned);
        self
    }

    fn store(&self) -> &Mutex<SHAMapStore> {
        &self.inner.store
    }

    /// Pruned-store metrics for RPC/CLI (`get_counts`, `server_info`), or
    /// `None` when this store is not the fjall pruned path.
    pub fn pruned_metrics(&self) -> Option<nodestore::PrunedMetrics> {
        self.inner.pruned.as_ref().map(|driver| driver.metrics())
    }

    pub fn snapshot(&self) -> SHAMapStore {
        self.store()
            .lock()
            .expect("shamap store mutex must not be poisoned")
            .clone()
    }

    pub fn on_ledger_closed(&self, ledger: Arc<Ledger>) {
        // fjall path: claim the validated ledger into the pruned index and
        // advance pruning. A failure here is logged and swallowed; the index
        // batch is atomic, so a failed claim is retried as a gap on the next
        // ledger rather than corrupting state.
        if let Some(pruned) = &self.inner.pruned {
            // Hand off to the pruned-store worker: the validated-ledger publish
            // path must not wait on diffing, pruning or fsync.
            pruned.submit(Arc::clone(&ledger));
        }
        self.store()
            .lock()
            .expect("shamap store mutex must not be poisoned")
            .on_ledger_closed(ledger);
    }

    pub fn rendezvous(&self) -> bool {
        self.store()
            .lock()
            .expect("shamap store mutex must not be poisoned")
            .rendezvous()
    }

    pub fn get_last_rotated(&self) -> u32 {
        self.store()
            .lock()
            .expect("shamap store mutex must not be poisoned")
            .get_last_rotated()
    }

    pub fn get_can_delete(&self) -> u32 {
        self.store()
            .lock()
            .expect("shamap store mutex must not be poisoned")
            .get_can_delete()
    }

    pub fn advisory_delete(&self) -> bool {
        self.store()
            .lock()
            .expect("shamap store mutex must not be poisoned")
            .advisory_delete()
    }

    pub fn set_can_delete(&self, can_delete: u32) -> Result<u32, String> {
        let mut store = self
            .store()
            .lock()
            .expect("shamap store mutex must not be poisoned");
        let can_delete = store.set_can_delete(can_delete);
        let effective_can_delete = store.get_can_delete();
        if store.advisory_delete() {
            if let Some(state_db) = &self.inner.state_db {
                state_db.set_can_delete(can_delete)?;
            }
        }
        self.inner
            .can_delete
            .store(effective_can_delete, Ordering::Release);
        self.publish_can_delete(effective_can_delete);
        Ok(can_delete)
    }

    /// Push the effective advisory cap into the pruned driver so the next
    /// prune target honours it immediately.
    fn publish_can_delete(&self, effective_can_delete: u32) {
        if let Some(pruned) = &self.inner.pruned {
            pruned
                .can_delete_handle()
                .store(effective_can_delete, Ordering::Release);
        }
    }

    pub fn saved_state(&self) -> SHAMapStoreSavedState {
        self.store()
            .lock()
            .expect("shamap store mutex must not be poisoned")
            .saved_state()
            .clone()
    }
}

impl ManagedComponent for SHAMapStoreComponent {
    fn start(&self) -> Result<(), String> {
        let mut store = self
            .store()
            .lock()
            .expect("shamap store mutex must not be poisoned");
        if store.advisory_delete() {
            if let Some(state_db) = &self.inner.state_db {
                let can_delete = state_db.get_can_delete()?;
                store.set_can_delete(can_delete);
            }
        }
        self.inner
            .can_delete
            .store(store.get_can_delete(), Ordering::Release);
        self.publish_can_delete(store.get_can_delete());
        if let Some(pruned) = &self.inner.pruned {
            pruned.spawn_worker();
        }
        // `store.start` is a no-op for the single/pruned stores (delete_interval
        // is zero): the rotating online-delete worker was removed, and the
        // fjall path prunes through `on_ledger_closed` -> PrunedDriver instead.
        let mut runtime = self
            .inner
            .runtime
            .lock()
            .expect("shamap store runtime mutex must not be poisoned");
        let _ = store.start(runtime.as_mut());
        Ok(())
    }

    fn stop(&self) {
        if let Some(pruned) = &self.inner.pruned {
            pruned.stop_worker();
        }
        let mut store = self
            .store()
            .lock()
            .expect("shamap store mutex must not be poisoned");
        let _ = store.request_stop();
        let mut runtime = self
            .inner
            .runtime
            .lock()
            .expect("shamap store runtime mutex must not be poisoned");
        let _ = store.stop(runtime.as_mut());
    }

    fn fd_required(&self) -> usize {
        self.store()
            .lock()
            .expect("shamap store mutex must not be poisoned")
            .fd_required() as usize
    }
}

#[cfg(test)]
mod tests {
    use super::{SHAMapStoreComponent, SHAMapStoreComponentRuntime};
    use crate::runtime::main_runtime::ManagedComponent;
    use crate::{
        SHAMapStore, SHAMapStoreHealthRuntime, SHAMapStoreOperatingMode, SHAMapStoreRuntime,
        SHAMapStoreSavedStateDb,
    };
    use basics::basic_config::BasicConfig;
    use ledger::Ledger;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;

    /// A minimal runtime: the rotating worker is gone, so the component only
    /// needs the two base runtime surfaces its lifecycle touches.
    #[derive(Default)]
    struct TestRuntime;

    impl SHAMapStoreRuntime for TestRuntime {
        fn start_background_work(&mut self) {}
        fn stop_background_work(&mut self) {}
        fn minimum_sql_seq(&self) -> Option<u32> {
            None
        }
    }

    impl SHAMapStoreHealthRuntime for TestRuntime {
        fn is_stopping(&self) -> bool {
            false
        }
        fn operating_mode(&self) -> SHAMapStoreOperatingMode {
            SHAMapStoreOperatingMode::Full
        }
        fn validated_ledger_age(&self) -> Duration {
            Duration::ZERO
        }
    }

    impl SHAMapStoreComponentRuntime for TestRuntime {}

    fn state_db() -> (TempDir, SHAMapStoreSavedStateDb) {
        let dir = TempDir::new().expect("tempdir");
        let mut config = BasicConfig::new();
        config.set_legacy("database_path", dir.path().to_string_lossy());
        let db = SHAMapStoreSavedStateDb::open(&config, "state").expect("state db");
        (dir, db)
    }

    #[test]
    fn lifecycle_is_a_noop_for_a_single_store_and_passes_ledgers_through() {
        let component =
            SHAMapStoreComponent::new(SHAMapStore::new(256, false, 7), Box::new(TestRuntime), None);
        // start/stop are no-ops (delete_interval == 0, no worker thread).
        component.start().expect("start");
        component.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
            900, 0, false,
        )));
        assert_eq!(component.fd_required(), 7);
        component.stop();
    }

    #[test]
    fn start_loads_advisory_delete_state_from_db() {
        let (_dir, state_db) = state_db();
        state_db.set_can_delete(777).expect("set can delete");
        let component = SHAMapStoreComponent::new(
            SHAMapStore::new(256, true, 11),
            Box::new(TestRuntime),
            Some(state_db),
        );
        component.start().expect("start");
        assert_eq!(component.get_can_delete(), 777);
        component.stop();
    }

    #[test]
    fn pruned_metrics_reflect_attached_driver() {
        use crate::shamap::pruned_driver::PrunedDriver;
        use nodestore::{Factory, MemoryFactory, NodeObject, NullJournal, PrunedConfig};

        let bare =
            SHAMapStoreComponent::new(SHAMapStore::new(256, false, 1), Box::new(TestRuntime), None);
        assert!(
            bare.pruned_metrics().is_none(),
            "a store with no pruned driver reports no pruned metrics"
        );

        let mut section = basics::basic_config::Section::new("node_db");
        section.set("type", "Memory");
        section.set("path", "component-pruned-metrics");
        let backend = MemoryFactory::new()
            .create_instance(
                NodeObject::KEY_BYTES,
                &section,
                0,
                Arc::new(nodestore::DummyScheduler),
                Arc::new(NullJournal),
            )
            .expect("memory backend");
        let backend: Arc<dyn nodestore::Backend> = Arc::from(backend);
        backend.open(true).expect("open backend");
        let driver =
            Arc::new(PrunedDriver::open(backend, PrunedConfig::default()).expect("driver opens"));

        let component =
            SHAMapStoreComponent::new(SHAMapStore::new(256, false, 1), Box::new(TestRuntime), None)
                .with_pruned_driver(driver);
        let metrics = component
            .pruned_metrics()
            .expect("attached driver reports metrics");
        assert_eq!(metrics.claimed_seq, None, "nothing claimed yet");
        assert_eq!(metrics.pruned_to, 0);
    }
}
