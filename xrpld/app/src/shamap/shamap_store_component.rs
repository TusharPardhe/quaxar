use crate::runtime::main_runtime::ManagedComponent;
use crate::shamap::shamap_store_app_runtime::SHAMapStoreRotationWindow;
use crate::{
    SHAMapStore, SHAMapStoreCopyDisposition, SHAMapStoreHealthPolicy, SHAMapStoreHealthRuntime,
    SHAMapStoreRuntime, SHAMapStoreSavedState, SHAMapStoreSavedStateDb, SHAMapStoreWorkerStep,
    run_shamap_store_worker_step_with_policy_refresh_and_stop,
};
use ledger::Ledger;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

struct NoopRotationWindow;

impl SHAMapStoreRotationWindow for NoopRotationWindow {}

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

    fn begin_rotation_window(&mut self) -> Result<Box<dyn SHAMapStoreRotationWindow>, String> {
        Ok(Box::new(NoopRotationWindow))
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
    /// Mirrors rippled's atomic `canDelete_`: maintenance observes the newest
    /// policy at its next destructive boundary without taking the producer
    /// store mutex.
    can_delete: AtomicU32,
    /// Cancellation observed by the detached worker's health wait. This must
    /// be published before joining because the worker owns the runtime mutex
    /// for the duration of a maintenance step.
    stopping: AtomicBool,
    /// Serializes worker snapshots from the managed worker and direct test or
    /// operator drains. Producers never take this lease, so a health-blocked
    /// online-delete worker cannot delay validation notification.
    worker_execution: Mutex<()>,
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
        let can_delete = store.get_can_delete();
        Self {
            inner: Arc::new(SHAMapStoreComponentInner {
                store: Mutex::new(store),
                runtime: Mutex::new(runtime),
                state_db: state_db.map(Arc::new),
                wake_mutex: Mutex::new(false),
                wake_condvar: Condvar::new(),
                can_delete: AtomicU32::new(can_delete),
                stopping: AtomicBool::new(false),
                worker_execution: Mutex::new(()),
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
        Ok(can_delete)
    }

    pub fn process_queued_ledger(&self) -> Result<Option<SHAMapStoreWorkerStep>, String> {
        run_detached_worker_step(&self.inner)
    }

    pub fn saved_state(&self) -> SHAMapStoreSavedState {
        self.store()
            .lock()
            .expect("shamap store mutex must not be poisoned")
            .saved_state()
            .clone()
    }
}

/// Execute one detached maintenance snapshot. The exclusive worker lease keeps
/// snapshot state linear while the shared store mutex remains free for
/// `on_ledger_closed` and policy updates.
fn run_detached_worker_step(
    inner: &Arc<SHAMapStoreComponentInner>,
) -> Result<Option<SHAMapStoreWorkerStep>, String> {
    let _execution = inner
        .worker_execution
        .lock()
        .expect("shamap store worker execution mutex must not be poisoned");
    let Some(mut worker_store) = inner
        .store
        .lock()
        .expect("shamap store mutex must not be poisoned")
        .take_worker_snapshot()
    else {
        return Ok(None);
    };
    let result = {
        let mut runtime = inner
            .runtime
            .lock()
            .expect("shamap store runtime mutex must not be poisoned");
        run_shamap_store_worker_step_with_policy_refresh_and_stop(
            &mut worker_store,
            runtime.as_mut(),
            inner.state_db.as_ref(),
            |worker_store| {
                // Rippled's `canDelete_` is atomic and its maintenance worker
                // reads it at the ready-to-delete boundary. Preserve that
                // nonblocking policy model rather than taking `store` while
                // runtime maintenance is active.
                worker_store.set_can_delete(inner.can_delete.load(Ordering::Acquire));
            },
            || inner.stopping.load(Ordering::Acquire),
        )
    };
    inner
        .store
        .lock()
        .expect("shamap store mutex must not be poisoned")
        .merge_worker_state(&worker_store);
    result
}

impl ManagedComponent for SHAMapStoreComponent {
    fn start(&self) -> Result<(), String> {
        self.inner.stopping.store(false, Ordering::Release);
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

                        let _ = run_detached_worker_step(&inner);
                    }
                }));
            }
        }
        Ok(())
    }

    fn stop(&self) {
        // Match rippled SHAMapStoreImp::stop(): make the exact predicate used
        // by healthWait observe shutdown before waiting for the worker join.
        // Calling runtime.stop_background_work() here would deadlock because
        // the detached worker holds the runtime mutex while health-waiting.
        self.inner.stopping.store(true, Ordering::Release);
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Condvar, Mutex};
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

    struct BlockingHealthRuntime {
        gate: Arc<(Mutex<(bool, bool)>, Condvar)>,
        clear_prior_calls: Arc<AtomicUsize>,
    }

    impl SHAMapStoreRuntime for BlockingHealthRuntime {
        fn start_background_work(&mut self) {}

        fn stop_background_work(&mut self) {}

        fn minimum_sql_seq(&self) -> Option<u32> {
            None
        }
    }

    impl SHAMapStoreHealthRuntime for BlockingHealthRuntime {
        fn is_stopping(&self) -> bool {
            false
        }

        fn operating_mode(&self) -> SHAMapStoreOperatingMode {
            if self.gate.0.lock().expect("blocking health gate").1 {
                SHAMapStoreOperatingMode::Full
            } else {
                SHAMapStoreOperatingMode::Other
            }
        }

        fn validated_ledger_age(&self) -> Duration {
            Duration::ZERO
        }
    }

    impl super::SHAMapStoreComponentRuntime for BlockingHealthRuntime {
        fn sleep(&mut self, _duration: Duration) {
            let (lock, wake) = &*self.gate;
            let mut state = lock.lock().expect("blocking health gate");
            state.0 = true;
            wake.notify_all();
            while !state.1 {
                state = wake.wait(state).expect("blocking health wait");
            }
        }

        fn clear_prior(&mut self, _last_rotated: u32) -> Result<(), String> {
            self.clear_prior_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    struct StuckHealthRuntime {
        entered_health_wait: Arc<AtomicBool>,
        stops: Arc<AtomicUsize>,
    }

    impl SHAMapStoreRuntime for StuckHealthRuntime {
        fn start_background_work(&mut self) {}

        fn stop_background_work(&mut self) {
            self.stops.fetch_add(1, Ordering::Relaxed);
        }

        fn minimum_sql_seq(&self) -> Option<u32> {
            None
        }
    }

    impl SHAMapStoreHealthRuntime for StuckHealthRuntime {
        fn is_stopping(&self) -> bool {
            false
        }

        fn operating_mode(&self) -> SHAMapStoreOperatingMode {
            SHAMapStoreOperatingMode::Other
        }

        fn validated_ledger_age(&self) -> Duration {
            Duration::ZERO
        }
    }

    impl super::SHAMapStoreComponentRuntime for StuckHealthRuntime {
        fn sleep(&mut self, _duration: Duration) {
            self.entered_health_wait.store(true, Ordering::Release);
            thread::yield_now();
        }
    }

    #[test]
    fn stop_cancels_worker_blocked_in_health_wait_before_joining() {
        let mut config = BasicConfig::new();
        config.section_mut("node_db").set("online_delete", "8");
        config
            .section_mut("node_db")
            .set("recovery_wait_seconds", "0");
        let store = SHAMapStore::from_config(&config, true, 8, 0).expect("store config");
        let entered_health_wait = Arc::new(AtomicBool::new(false));
        let stops = Arc::new(AtomicUsize::new(0));
        let component = Arc::new(SHAMapStoreComponent::new(
            store,
            Box::new(StuckHealthRuntime {
                entered_health_wait: Arc::clone(&entered_health_wait),
                stops: Arc::clone(&stops),
            }),
            None,
        ));
        component.start().expect("component start");

        component.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
            100, 0, false,
        )));
        wait_for(|| component.rendezvous());
        component.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
            108, 0, false,
        )));
        wait_for(|| entered_health_wait.load(Ordering::Acquire));

        let (stopped_tx, stopped_rx) = std::sync::mpsc::channel();
        let stopping_component = Arc::clone(&component);
        let stop_thread = thread::spawn(move || {
            stopping_component.stop();
            stopped_tx.send(()).expect("stop completion");
        });
        stopped_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("health-waiting worker must observe cancellation before join");
        stop_thread.join().expect("stop thread");

        assert_eq!(stops.load(Ordering::Relaxed), 1);
        assert!(component.rendezvous());
    }

    #[test]
    fn ledger_notification_does_not_wait_for_a_health_blocked_worker() {
        let gate = Arc::new((Mutex::new((false, false)), Condvar::new()));
        let clear_prior_calls = Arc::new(AtomicUsize::new(0));
        let component = Arc::new(SHAMapStoreComponent::new(
            SHAMapStore::new(10, true, 0),
            Box::new(BlockingHealthRuntime {
                gate: Arc::clone(&gate),
                clear_prior_calls: Arc::clone(&clear_prior_calls),
            }),
            None,
        ));

        // Establish the rotation baseline. Sequence 110 is eligible to rotate
        // from 100, then stalls in healthWait exactly like the production
        // online-delete worker.
        component.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
            100, 0, false,
        )));
        component
            .process_queued_ledger()
            .expect("baseline processing")
            .expect("baseline ledger");

        component.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
            110, 0, false,
        )));
        let worker = {
            let component = Arc::clone(&component);
            thread::spawn(move || component.process_queued_ledger())
        };
        {
            let (lock, wake) = &*gate;
            let mut state = lock.lock().expect("blocking health gate");
            while !state.0 {
                state = wake
                    .wait_timeout(state, Duration::from_secs(1))
                    .expect("blocking health entry wait")
                    .0;
                assert!(state.0, "worker should enter the health wait");
            }
        }

        let (notified_tx, notified_rx) = std::sync::mpsc::channel();
        let notification = {
            let component = Arc::clone(&component);
            thread::spawn(move || {
                component.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
                    111, 0, false,
                )));
                notified_tx.send(()).expect("notification result");
            })
        };
        notified_rx
            .recv_timeout(Duration::from_millis(200))
            .expect("ledger notification must not block behind maintenance");

        // Revoke the advisory deletion permission while the detached worker
        // sleeps. The worker must refresh this live value before clear_prior.
        component.set_can_delete(98).expect("revoke can_delete");
        let queued_worker = {
            let component = Arc::clone(&component);
            thread::spawn(move || component.process_queued_ledger())
        };

        {
            let (lock, wake) = &*gate;
            let mut state = lock.lock().expect("blocking health gate");
            state.1 = true;
            wake.notify_all();
        }
        worker.join().expect("worker join").expect("worker result");
        queued_worker
            .join()
            .expect("queued worker join")
            .expect("queued worker result");
        notification.join().expect("notification join");

        assert_eq!(clear_prior_calls.load(Ordering::Relaxed), 0);
        assert_eq!(component.saved_state().last_rotated, 100);
        assert_eq!(component.snapshot().queued_ledger_seq(), None);
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
