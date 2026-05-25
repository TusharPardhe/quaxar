use app::{
    SHAMapStore, SHAMapStoreComponentRuntime, SHAMapStoreCopyDisposition, SHAMapStoreHealthPolicy,
    SHAMapStoreHealthRuntime, SHAMapStoreOperatingMode, SHAMapStoreRuntime, SHAMapStoreSavedState,
    SHAMapStoreWorkerStep, run_shamap_store_worker_step,
};
use basics::base_uint::Uint256;
use ledger::Ledger;
use std::sync::Arc;
use std::time::Duration;

struct RecordingRuntime {
    minimum_sql_seq: Option<u32>,
    stopping: bool,
    mode: SHAMapStoreOperatingMode,
    age: Duration,
    events: Vec<String>,
    rotate_result: (String, String),
    copy_result: SHAMapStoreCopyDisposition,
}

impl Default for RecordingRuntime {
    fn default() -> Self {
        Self {
            minimum_sql_seq: None,
            stopping: false,
            mode: SHAMapStoreOperatingMode::Full,
            age: Duration::from_secs(0),
            events: Vec::new(),
            rotate_result: (String::new(), String::new()),
            copy_result: SHAMapStoreCopyDisposition::Completed { node_count: 0 },
        }
    }
}

impl SHAMapStoreRuntime for RecordingRuntime {
    fn start_background_work(&mut self) {}

    fn stop_background_work(&mut self) {}

    fn minimum_sql_seq(&self) -> Option<u32> {
        self.minimum_sql_seq
    }
}

impl SHAMapStoreHealthRuntime for RecordingRuntime {
    fn is_stopping(&self) -> bool {
        self.stopping
    }

    fn operating_mode(&self) -> SHAMapStoreOperatingMode {
        self.mode
    }

    fn validated_ledger_age(&self) -> Duration {
        self.age
    }
}

impl SHAMapStoreComponentRuntime for RecordingRuntime {
    fn clear_prior(&mut self, last_rotated: u32) -> Result<(), String> {
        self.events.push(format!("clear_prior:{last_rotated}"));
        Ok(())
    }

    fn copy_validated_ledger(
        &mut self,
        validated_ledger: Arc<Ledger>,
        _health_policy: SHAMapStoreHealthPolicy,
    ) -> Result<SHAMapStoreCopyDisposition, String> {
        self.events
            .push(format!("copy:{}", validated_ledger.header().seq));
        Ok(self.copy_result)
    }

    fn freshen_caches(&mut self) -> Result<(), String> {
        self.events.push("freshen".to_owned());
        Ok(())
    }

    fn prepare_rotation(&mut self) -> Result<(), String> {
        self.events.push("prepare_rotation".to_owned());
        Ok(())
    }

    fn rotate_backends(&mut self) -> Result<(String, String), String> {
        self.events.push("rotate".to_owned());
        Ok(self.rotate_result.clone())
    }

    fn clear_caches(&mut self, validated_seq: u32) -> Result<(), String> {
        self.events.push(format!("clear_caches:{validated_seq}"));
        Ok(())
    }
}

fn healthy_runtime() -> RecordingRuntime {
    RecordingRuntime {
        minimum_sql_seq: Some(700),
        mode: SHAMapStoreOperatingMode::Full,
        age: Duration::from_secs(1),
        rotate_result: ("writable.next".to_owned(), "archive.prev".to_owned()),
        ..RecordingRuntime::default()
    }
}

#[test]
fn shamap_store_worker_initializes_first_validated_ledger_without_rotating() {
    let mut store = SHAMapStore::new(256, true, 0);
    store.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
        900, 0, false,
    )));
    let mut runtime = healthy_runtime();

    let step = run_shamap_store_worker_step(&mut store, &mut runtime, None)
        .expect("step")
        .expect("queued ledger");

    assert_eq!(
        step,
        SHAMapStoreWorkerStep {
            runloop: app::runloop_step(
                900,
                0,
                256,
                u32::MAX,
                app::SHAMapStoreHealthStatus::KeepGoing
            ),
            rotated: false,
            stopped: false,
            minimum_online: Some(700),
        }
    );
    assert_eq!(store.get_last_rotated(), 900);
    assert!(runtime.events.is_empty());
}

#[test]
fn shamap_store_worker_runs_rotation_steps_in() {
    let mut store = SHAMapStore::new(256, true, 0);
    store.set_saved_state(SHAMapStoreSavedState {
        writable_db: "writable.current".to_owned(),
        archive_db: "archive.current".to_owned(),
        last_rotated: 900,
    });
    store.set_can_delete(900);
    store.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_156, 0, false,
    )));
    let mut runtime = healthy_runtime();

    let step = run_shamap_store_worker_step(&mut store, &mut runtime, None)
        .expect("step")
        .expect("queued ledger");

    assert!(step.rotated);
    assert!(!step.stopped);
    assert_eq!(store.get_last_rotated(), 1_156);
    assert_eq!(store.minimum_online(&runtime), Some(901));
    assert_eq!(store.saved_state().writable_db, "writable.next");
    assert_eq!(store.saved_state().archive_db, "archive.prev");
    assert_eq!(
        runtime.events,
        vec![
            "clear_prior:900".to_owned(),
            "copy:1156".to_owned(),
            "freshen".to_owned(),
            "prepare_rotation".to_owned(),
            "clear_caches:1156".to_owned(),
            "rotate".to_owned(),
            "clear_caches:1156".to_owned(),
        ]
    );
}

#[test]
fn shamap_store_worker_stops_before_rotation_side_effects_when_health_wait_stops() {
    let mut store = SHAMapStore::new(256, true, 0);
    store.set_saved_state(SHAMapStoreSavedState {
        writable_db: "writable.current".to_owned(),
        archive_db: "archive.current".to_owned(),
        last_rotated: 900,
    });
    store.set_can_delete(900);
    store.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_156, 0, false,
    )));
    let mut runtime = healthy_runtime();
    runtime.stopping = true;

    let step = run_shamap_store_worker_step(&mut store, &mut runtime, None)
        .expect("step")
        .expect("queued ledger");

    assert!(!step.rotated);
    assert!(step.stopped);
    assert_eq!(store.get_last_rotated(), 900);
    assert!(runtime.events.is_empty());
}

#[test]
fn shamap_store_worker_skips_rotation_when_copy_hits_missing_node() {
    let mut store = SHAMapStore::new(256, true, 0);
    store.set_saved_state(SHAMapStoreSavedState {
        writable_db: "writable.current".to_owned(),
        archive_db: "archive.current".to_owned(),
        last_rotated: 900,
    });
    store.set_can_delete(900);
    store.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_156, 0, false,
    )));
    let mut runtime = healthy_runtime();
    runtime.copy_result = SHAMapStoreCopyDisposition::MissingNode {
        hash: Uint256::from_array([0xAB; 32]),
        node_count: 17,
    };

    let step = run_shamap_store_worker_step(&mut store, &mut runtime, None)
        .expect("step")
        .expect("queued ledger");

    assert!(!step.rotated);
    assert!(!step.stopped);
    assert_eq!(store.get_last_rotated(), 900);
    assert_eq!(store.saved_state().writable_db, "writable.current");
    assert_eq!(
        runtime.events,
        vec!["clear_prior:900".to_owned(), "copy:1156".to_owned()]
    );
}
