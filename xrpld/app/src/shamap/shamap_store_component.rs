use crate::runtime::main_runtime::ManagedComponent;
use crate::{
    SHAMapStore, SHAMapStoreCopyDisposition, SHAMapStoreHealthPolicy, SHAMapStoreHealthRuntime,
    SHAMapStoreRuntime, SHAMapStoreSavedState, SHAMapStoreSavedStateDb, SHAMapStoreWorkerStep,
    run_shamap_store_worker_step,
};
use ledger::Ledger;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub trait SHAMapStoreComponentRuntime:
    SHAMapStoreRuntime + SHAMapStoreHealthRuntime + Send + 'static
{
    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn clear_prior(&mut self, _last_rotated: u32) -> Result<(), String> {
        Ok(())
    }

    fn copy_validated_ledger(
        &mut self,
        _validated_ledger: Arc<Ledger>,
        _health_policy: SHAMapStoreHealthPolicy,
    ) -> Result<SHAMapStoreCopyDisposition, String> {
        Ok(SHAMapStoreCopyDisposition::Completed { node_count: 0 })
    }

    fn freshen_caches(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn prepare_rotation(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn rotate_backends(&mut self) -> Result<(String, String), String> {
        Ok((String::new(), String::new()))
    }

    fn clear_caches(&mut self, _validated_seq: u32) -> Result<(), String> {
        Ok(())
    }
}

struct SHAMapStoreComponentInner {
    store: Mutex<SHAMapStore>,
    runtime: Mutex<Box<dyn SHAMapStoreComponentRuntime>>,
    state_db: Option<Arc<SHAMapStoreSavedStateDb>>,
    wake_mutex: Mutex<bool>,
    wake_condvar: Condvar,
}

pub struct SHAMapStoreComponent {
    inner: Arc<SHAMapStoreComponentInner>,
    worker: Mutex<Option<JoinHandle<()>>>,
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
        Self {
            inner: Arc::new(SHAMapStoreComponentInner {
                store: Mutex::new(store),
                runtime: Mutex::new(runtime),
                state_db: state_db.map(Arc::new),
                wake_mutex: Mutex::new(false),
                wake_condvar: Condvar::new(),
            }),
            worker: Mutex::new(None),
        }
    }

    fn store(&self) -> &Mutex<SHAMapStore> {
        &self.inner.store
    }

    pub fn snapshot(&self) -> SHAMapStore {
        self.store()
            .lock()
            .expect("shamap store mutex must not be poisoned")
            .clone()
    }

    pub fn on_ledger_closed(&self, ledger: Arc<Ledger>) {
        self.store()
            .lock()
            .expect("shamap store mutex must not be poisoned")
            .on_ledger_closed(ledger);
        let mut wake = self
            .inner
            .wake_mutex
            .lock()
            .expect("shamap store wake mutex must not be poisoned");
        *wake = true;
        self.inner.wake_condvar.notify_one();
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

    pub fn set_can_delete(&self, can_delete: u32) -> Result<u32, String> {
        let mut store = self
            .store()
            .lock()
            .expect("shamap store mutex must not be poisoned");
        let can_delete = store.set_can_delete(can_delete);
        if store.advisory_delete() {
            if let Some(state_db) = &self.inner.state_db {
                state_db.set_can_delete(can_delete)?;
            }
        }
        Ok(can_delete)
    }

    pub fn process_queued_ledger(&self) -> Result<Option<SHAMapStoreWorkerStep>, String> {
        let mut store = self
            .store()
            .lock()
            .expect("shamap store mutex must not be poisoned");
        let mut runtime = self
            .inner
            .runtime
            .lock()
            .expect("shamap store runtime mutex must not be poisoned");
        run_shamap_store_worker_step(&mut store, runtime.as_mut(), self.inner.state_db.as_ref())
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
        let mut runtime = self
            .inner
            .runtime
            .lock()
            .expect("shamap store runtime mutex must not be poisoned");
        let started = store.start(runtime.as_mut());
        drop(runtime);
        drop(store);
        if started {
            let mut worker = self
                .worker
                .lock()
                .expect("shamap store worker mutex must not be poisoned");
            if worker.is_none() {
                let inner = Arc::clone(&self.inner);
                *worker = Some(std::thread::spawn(move || {
                    loop {
                        let mut wake = inner
                            .wake_mutex
                            .lock()
                            .expect("shamap store wake mutex must not be poisoned");
                        while !*wake {
                            let stopping = inner
                                .store
                                .lock()
                                .expect("shamap store mutex must not be poisoned")
                                .is_stopping();
                            let has_work = inner
                                .store
                                .lock()
                                .expect("shamap store mutex must not be poisoned")
                                .queued_ledger_seq()
                                .is_some();
                            if stopping || has_work {
                                break;
                            }
                            wake = inner
                                .wake_condvar
                                .wait(wake)
                                .expect("shamap store wake condvar must not be poisoned");
                        }
                        *wake = false;
                        drop(wake);

                        let stopping = inner
                            .store
                            .lock()
                            .expect("shamap store mutex must not be poisoned")
                            .is_stopping();
                        if stopping {
                            inner
                                .store
                                .lock()
                                .expect("shamap store mutex must not be poisoned")
                                .finish_rendezvous();
                            return;
                        }

                        let mut store = inner
                            .store
                            .lock()
                            .expect("shamap store mutex must not be poisoned");
                        let mut runtime = inner
                            .runtime
                            .lock()
                            .expect("shamap store runtime mutex must not be poisoned");
                        let _ = run_shamap_store_worker_step(
                            &mut store,
                            runtime.as_mut(),
                            inner.state_db.as_ref(),
                        );
                    }
                }));
            }
        }
        Ok(())
    }

    fn stop(&self) {
        let mut store = self
            .store()
            .lock()
            .expect("shamap store mutex must not be poisoned");
        let stopped = store.request_stop();
        drop(store);
        if stopped {
            let mut wake = self
                .inner
                .wake_mutex
                .lock()
                .expect("shamap store wake mutex must not be poisoned");
            *wake = true;
            self.inner.wake_condvar.notify_one();
            drop(wake);
            if let Some(handle) = self
                .worker
                .lock()
                .expect("shamap store worker mutex must not be poisoned")
                .take()
            {
                let _ = handle.join();
            }
            let mut store = self
                .store()
                .lock()
                .expect("shamap store mutex must not be poisoned");
            let mut runtime = self
                .inner
                .runtime
                .lock()
                .expect("shamap store runtime mutex must not be poisoned");
            let _ = store.stop(runtime.as_mut());
        }
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
    use super::SHAMapStoreComponent;
    use crate::runtime::main_runtime::ManagedComponent;
    use crate::{
        SHAMapStore, SHAMapStoreHealthRuntime, SHAMapStoreOperatingMode, SHAMapStoreRuntime,
        SHAMapStoreSavedStateDb,
    };
    use basics::basic_config::BasicConfig;
    use ledger::Ledger;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir;

    #[derive(Default, Debug)]
    struct SharedRuntimeState {
        starts: AtomicUsize,
        stops: AtomicUsize,
        sleeps: Mutex<Vec<Duration>>,
        clear_prior: Mutex<Vec<u32>>,
        copied_ledgers: Mutex<Vec<u32>>,
        freshen_calls: AtomicUsize,
        prepare_calls: AtomicUsize,
        rotate_calls: AtomicUsize,
        clear_caches: Mutex<Vec<u32>>,
    }

    struct Runtime {
        shared: Arc<SharedRuntimeState>,
        wait_cycles: Arc<AtomicUsize>,
        rotate_names: (String, String),
    }

    impl Runtime {
        fn new(
            shared: Arc<SharedRuntimeState>,
            wait_cycles: Arc<AtomicUsize>,
            rotate_names: (String, String),
        ) -> Self {
            Self {
                shared,
                wait_cycles,
                rotate_names,
            }
        }
    }

    impl SHAMapStoreRuntime for Runtime {
        fn start_background_work(&mut self) {
            self.shared.starts.fetch_add(1, Ordering::Relaxed);
        }

        fn stop_background_work(&mut self) {
            self.shared.stops.fetch_add(1, Ordering::Relaxed);
        }

        fn minimum_sql_seq(&self) -> Option<u32> {
            Some(500)
        }
    }

    impl SHAMapStoreHealthRuntime for Runtime {
        fn is_stopping(&self) -> bool {
            false
        }

        fn operating_mode(&self) -> SHAMapStoreOperatingMode {
            if self.wait_cycles.load(Ordering::Relaxed) == 0 {
                SHAMapStoreOperatingMode::Other
            } else {
                SHAMapStoreOperatingMode::Full
            }
        }

        fn validated_ledger_age(&self) -> Duration {
            Duration::from_secs(1)
        }
    }

    impl super::SHAMapStoreComponentRuntime for Runtime {
        fn sleep(&mut self, duration: Duration) {
            self.shared
                .sleeps
                .lock()
                .expect("sleep mutex must not be poisoned")
                .push(duration);
            self.wait_cycles.fetch_add(1, Ordering::Relaxed);
        }

        fn clear_prior(&mut self, last_rotated: u32) -> Result<(), String> {
            self.shared
                .clear_prior
                .lock()
                .expect("clear_prior mutex must not be poisoned")
                .push(last_rotated);
            Ok(())
        }

        fn copy_validated_ledger(
            &mut self,
            validated_ledger: Arc<Ledger>,
            _health_policy: crate::SHAMapStoreHealthPolicy,
        ) -> Result<crate::SHAMapStoreCopyDisposition, String> {
            self.shared
                .copied_ledgers
                .lock()
                .expect("copied_ledgers mutex must not be poisoned")
                .push(validated_ledger.header().seq);
            Ok(crate::SHAMapStoreCopyDisposition::Completed { node_count: 3 })
        }

        fn freshen_caches(&mut self) -> Result<(), String> {
            self.shared.freshen_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        fn prepare_rotation(&mut self) -> Result<(), String> {
            self.shared.prepare_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        fn rotate_backends(&mut self) -> Result<(String, String), String> {
            self.shared.rotate_calls.fetch_add(1, Ordering::Relaxed);
            Ok(self.rotate_names.clone())
        }

        fn clear_caches(&mut self, validated_seq: u32) -> Result<(), String> {
            self.shared
                .clear_caches
                .lock()
                .expect("clear_caches mutex must not be poisoned")
                .push(validated_seq);
            Ok(())
        }
    }

    fn default_runtime() -> (Arc<SharedRuntimeState>, Box<Runtime>) {
        let shared = Arc::new(SharedRuntimeState::default());
        let runtime = Box::new(Runtime::new(
            Arc::clone(&shared),
            Arc::new(AtomicUsize::new(1)),
            (String::new(), String::new()),
        ));
        (shared, runtime)
    }

    fn state_db() -> (TempDir, SHAMapStoreSavedStateDb) {
        let dir = TempDir::new().expect("tempdir");
        let mut config = BasicConfig::new();
        config.set_legacy("database_path", dir.path().to_string_lossy());
        let db = SHAMapStoreSavedStateDb::open(&config, "state").expect("state db");
        (dir, db)
    }

    fn wait_for(condition: impl Fn() -> bool) {
        for _ in 0..50 {
            if condition() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(condition(), "condition should become true");
    }

    #[test]
    fn shamap_store_component_runs_managed_lifecycle_and_rendezvous() {
        let (shared, runtime) = default_runtime();
        let component = SHAMapStoreComponent::new(SHAMapStore::new(256, false, 7), runtime, None);

        component.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
            900, 0, false,
        )));
        assert!(!component.rendezvous());
        let step = component
            .process_queued_ledger()
            .expect("step")
            .expect("queued work");
        assert_eq!(step.runloop.decision.last_rotated, 900);
        assert!(component.rendezvous());
        assert_eq!(component.fd_required(), 7);
        component.stop();
        assert_eq!(shared.starts.load(Ordering::Relaxed), 0);
        assert_eq!(shared.stops.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn shamap_store_component_start_loads_advisory_delete_state_from_db() {
        let (_dir, state_db) = state_db();
        state_db.set_can_delete(777).expect("set can delete");

        let (shared, runtime) = default_runtime();
        let component =
            SHAMapStoreComponent::new(SHAMapStore::new(256, true, 11), runtime, Some(state_db));

        component.start().expect("component start");
        assert_eq!(component.get_can_delete(), 777);
        assert_eq!(component.fd_required(), 11);
        component.stop();

        assert_eq!(shared.starts.load(Ordering::Relaxed), 1);
        assert_eq!(shared.stops.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn shamap_store_component_worker_rotates_and_persists_state_after_health_wait() {
        let (_dir, state_db) = state_db();
        let shared = Arc::new(SharedRuntimeState::default());
        let component = SHAMapStoreComponent::new(
            SHAMapStore::new(256, true, 9),
            Box::new(Runtime::new(
                Arc::clone(&shared),
                Arc::new(AtomicUsize::new(0)),
                ("writable.next".to_owned(), "archive.prev".to_owned()),
            )),
            Some(state_db),
        );

        component.start().expect("component start");
        component.set_can_delete(900).expect("set can delete");

        component.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
            900, 0, false,
        )));
        wait_for(|| component.rendezvous());
        assert_eq!(component.get_last_rotated(), 900);

        component.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
            1_156, 0, false,
        )));
        wait_for(|| component.rendezvous());

        let saved = component.saved_state();
        assert_eq!(saved.writable_db, "writable.next");
        assert_eq!(saved.archive_db, "archive.prev");
        assert_eq!(saved.last_rotated, 1_156);
        assert_eq!(component.get_last_rotated(), 1_156);

        assert_eq!(
            shared
                .sleeps
                .lock()
                .expect("sleep mutex must not be poisoned")
                .as_slice(),
            &[Duration::from_secs(5)]
        );
        assert_eq!(
            shared
                .clear_prior
                .lock()
                .expect("clear_prior mutex must not be poisoned")
                .as_slice(),
            &[900]
        );
        assert_eq!(
            shared
                .copied_ledgers
                .lock()
                .expect("copied_ledgers mutex must not be poisoned")
                .as_slice(),
            &[1_156]
        );
        assert_eq!(shared.freshen_calls.load(Ordering::Relaxed), 1);
        assert_eq!(shared.prepare_calls.load(Ordering::Relaxed), 1);
        assert_eq!(shared.rotate_calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            shared
                .clear_caches
                .lock()
                .expect("clear_caches mutex must not be poisoned")
                .as_slice(),
            &[1_156, 1_156]
        );

        component.stop();
        assert_eq!(shared.starts.load(Ordering::Relaxed), 1);
        assert_eq!(shared.stops.load(Ordering::Relaxed), 1);
    }
}
