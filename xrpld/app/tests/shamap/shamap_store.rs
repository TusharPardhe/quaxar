use app::{
    ApplicationRoot, MainRuntime, SHAMapStore, SHAMapStoreComponent, SHAMapStoreComponentRuntime,
    SHAMapStoreConfig, SHAMapStoreHealthRuntime, SHAMapStoreHealthStatus, SHAMapStoreOperatingMode,
    SHAMapStoreRuntime, SHAMapStoreSavedState,
};
use basics::basic_config::BasicConfig;
use ledger::Ledger;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

#[derive(Default)]
struct Runtime {
    minimum_sql_seq: Option<u32>,
}

impl SHAMapStoreRuntime for Runtime {
    fn start_background_work(&mut self) {}

    fn stop_background_work(&mut self) {}

    fn minimum_sql_seq(&self) -> Option<u32> {
        self.minimum_sql_seq
    }
}

impl SHAMapStoreHealthRuntime for Runtime {
    fn is_stopping(&self) -> bool {
        false
    }

    fn operating_mode(&self) -> SHAMapStoreOperatingMode {
        SHAMapStoreOperatingMode::Full
    }

    fn validated_ledger_age(&self) -> Duration {
        Duration::from_secs(1)
    }
}

impl SHAMapStoreComponentRuntime for Runtime {}

#[derive(Default)]
struct ComponentRuntimeShared {
    starts: AtomicUsize,
    stops: AtomicUsize,
    copied: Mutex<Vec<u32>>,
}

struct ComponentRuntime {
    shared: Arc<ComponentRuntimeShared>,
}

impl SHAMapStoreRuntime for ComponentRuntime {
    fn start_background_work(&mut self) {
        self.shared.starts.fetch_add(1, Ordering::Relaxed);
    }

    fn stop_background_work(&mut self) {
        self.shared.stops.fetch_add(1, Ordering::Relaxed);
    }

    fn minimum_sql_seq(&self) -> Option<u32> {
        Some(700)
    }
}

impl SHAMapStoreHealthRuntime for ComponentRuntime {
    fn is_stopping(&self) -> bool {
        false
    }

    fn operating_mode(&self) -> SHAMapStoreOperatingMode {
        SHAMapStoreOperatingMode::Full
    }

    fn validated_ledger_age(&self) -> Duration {
        Duration::from_secs(1)
    }
}

impl SHAMapStoreComponentRuntime for ComponentRuntime {
    fn copy_validated_ledger(
        &mut self,
        validated_ledger: Arc<Ledger>,
        _health_policy: app::SHAMapStoreHealthPolicy,
    ) -> Result<app::SHAMapStoreCopyDisposition, String> {
        self.shared
            .copied
            .lock()
            .expect("copied mutex")
            .push(validated_ledger.header().seq);
        Ok(app::SHAMapStoreCopyDisposition::Completed { node_count: 1 })
    }
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
fn shamap_store_config_and_owner_state_match_current_rust_boundary() {
    let mut config = BasicConfig::new();
    let node_db = config.section_mut("node_db");
    node_db.set("type", "RocksDB");
    node_db.set("path", "/tmp/node_db");
    node_db.set("online_delete", "256");
    node_db.set("advisory_delete", "1");
    node_db.set("delete_batch", "500");

    let store = SHAMapStore::from_config(&config, false, 128, 32).expect("store");
    assert_eq!(
        store.config(),
        &SHAMapStoreConfig {
            delete_interval: 256,
            advisory_delete: true,
            delete_batch: 500,
            ..SHAMapStoreConfig::default()
        }
    );
    assert_eq!(store.fd_required(), 32);
}

#[test]
fn shamap_store_rotation_boundary_overrides_minimum_sql_seq() {
    let mut runtime = Runtime {
        minimum_sql_seq: Some(700),
    };
    let mut store = SHAMapStore::new(256, true, 0);

    assert_eq!(store.minimum_online(&runtime), Some(700));
    store.note_rotation_boundary(900);
    assert_eq!(store.minimum_online(&runtime), Some(901));

    store.set_saved_state(SHAMapStoreSavedState {
        writable_db: "writable".to_owned(),
        archive_db: "archive".to_owned(),
        last_rotated: 900,
    });
    let decision = store.rotation_decision(1156, SHAMapStoreHealthStatus::KeepGoing);
    assert!(decision.ready_to_rotate);

    runtime.minimum_sql_seq = Some(1_200);
    assert_eq!(store.minimum_online(&runtime), Some(901));
}

#[test]
fn shamap_store_component_updates_rotation_boundary_and_rendezvous() {
    let component = SHAMapStoreComponent::new(
        SHAMapStore::new(256, true, 9),
        Box::new(Runtime::default()),
        None,
    );
    component.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
        900, 0, false,
    )));

    let initial = component
        .process_queued_ledger()
        .expect("step")
        .expect("queued");
    assert_eq!(initial.runloop.decision.last_rotated, 900);
    assert!(component.rendezvous());

    component.set_can_delete(900).expect("can delete");
    component.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_156, 0, false,
    )));
    let rotated = component
        .process_queued_ledger()
        .expect("step")
        .expect("queued");
    assert!(rotated.runloop.decision.ready_to_rotate);
    assert_eq!(component.get_last_rotated(), 1_156);
    assert_eq!(component.saved_state().last_rotated, 1_156);
}

#[test]
fn runtime_drives_real_shamap_store_component_lifecycle() {
    let shared = Arc::new(ComponentRuntimeShared::default());
    let component = Arc::new(SHAMapStoreComponent::new(
        SHAMapStore::new(256, false, 9),
        Box::new(ComponentRuntime {
            shared: Arc::clone(&shared),
        }),
        None,
    ));

    let mut root = ApplicationRoot::new(0).expect("root");
    let _service = root.attach_shamap_store_component(component.clone());

    let runtime = MainRuntime::new(root);
    runtime.start().expect("runtime start");

    assert!(
        runtime
            .root()
            .on_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
                900, 0, false,
            )))
    );
    assert_eq!(runtime.root().validated_ledger_seq(), Some(900));
    wait_for(|| component.rendezvous());

    runtime.shutdown();

    assert_eq!(component.get_last_rotated(), 900);
    assert_eq!(shared.starts.load(Ordering::Relaxed), 1);
    assert_eq!(shared.stops.load(Ordering::Relaxed), 1);
    assert_eq!(
        shared.copied.lock().expect("copied mutex").as_slice(),
        &[] as &[u32]
    );
}
