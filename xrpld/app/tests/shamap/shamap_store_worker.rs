use app::{
    SHAMapStore, SHAMapStoreComponentRuntime, SHAMapStoreCopyDisposition, SHAMapStoreHealthPolicy,
    SHAMapStoreHealthRuntime, SHAMapStoreOperatingMode, SHAMapStoreRuntime, SHAMapStoreSavedState,
    SHAMapStoreSavedStateDb, SHAMapStoreWorkerStep, run_shamap_store_worker_step,
};
use basics::base_uint::Uint256;
use basics::basic_config::BasicConfig;
use ledger::Ledger;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;

struct RecordingRuntime {
    minimum_sql_seq: Option<u32>,
    stopping: bool,
    mode: SHAMapStoreOperatingMode,
    age: Duration,
    events: Vec<String>,
    rotate_result: (String, String),
    copy_result: SHAMapStoreCopyDisposition,
    validated_seq: AtomicU32,
    missing_ledgers: AtomicUsize,
    missing_before_seq: AtomicU32,
    complete_validated_tip: AtomicBool,
    advance_on_sleep: bool,
    introduce_gap_after_copy: bool,
    disconnect_after_copy: bool,
    introduce_gap_before_clear_caches: bool,
    disconnected: bool,
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
            validated_seq: AtomicU32::new(0),
            missing_ledgers: AtomicUsize::new(0),
            missing_before_seq: AtomicU32::new(0),
            complete_validated_tip: AtomicBool::new(true),
            advance_on_sleep: false,
            introduce_gap_after_copy: false,
            disconnect_after_copy: false,
            introduce_gap_before_clear_caches: false,
            disconnected: false,
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

    fn is_disconnected(&self) -> bool {
        self.disconnected
    }

    fn validated_ledger_seq(&self) -> Option<u32> {
        match self.validated_seq.load(Ordering::Relaxed) {
            0 => None,
            seq => Some(seq),
        }
    }

    fn missing_from_complete_ledger_range(&self, first: u32, last: u32) -> usize {
        let missing_before = self.missing_before_seq.load(Ordering::Relaxed);
        if missing_before != 0 && first <= missing_before && missing_before <= last {
            return 1;
        }
        self.missing_ledgers.load(Ordering::Relaxed)
    }

    fn has_complete_validated_ledger(&self, _seq: u32) -> bool {
        self.complete_validated_tip.load(Ordering::Relaxed)
    }
}

impl SHAMapStoreComponentRuntime for RecordingRuntime {
    fn sleep(&mut self, duration: Duration) {
        self.events.push(format!("sleep:{duration:?}"));
        if self.advance_on_sleep {
            self.validated_seq.fetch_add(1, Ordering::Relaxed);
        }
    }

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
        if self.introduce_gap_after_copy {
            self.missing_ledgers.store(1, Ordering::Relaxed);
        }
        if self.disconnect_after_copy {
            self.disconnected = true;
        }
        Ok(self.copy_result)
    }

    fn freshen_caches(&mut self) -> Result<(), String> {
        self.events.push("freshen".to_owned());
        Ok(())
    }

    fn prepare_rotation(&mut self) -> Result<(), String> {
        self.events.push("prepare_rotation".to_owned());
        if self.introduce_gap_before_clear_caches {
            self.missing_ledgers.store(1, Ordering::Relaxed);
        }
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

fn durable_state_db(initial: &SHAMapStoreSavedState) -> (TempDir, Arc<SHAMapStoreSavedStateDb>) {
    let directory = TempDir::new().expect("temporary state directory");
    let mut config = BasicConfig::new();
    config.set_legacy("database_path", directory.path().to_string_lossy());
    let state_db =
        Arc::new(SHAMapStoreSavedStateDb::open(&config, "rotation-state").expect("state database"));
    state_db.set_state(initial).expect("initial state");
    (directory, state_db)
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

#[test]
fn shamap_store_health_pauses_for_an_old_gap_and_distinguishes_a_building_tip() {
    let runtime = healthy_runtime();
    runtime.validated_seq.store(1_156, Ordering::Relaxed);
    runtime.missing_ledgers.store(1, Ordering::Relaxed);
    let policy = SHAMapStoreHealthPolicy {
        age_threshold: Duration::from_secs(60),
        recovery_wait: Duration::from_secs(10),
    };

    let old_gap = policy.evaluate_with_progress(&runtime, 900, 1_156);
    assert_eq!(
        old_gap.status,
        app::SHAMapStoreHealthStatus::Waiting(Duration::from_secs(10))
    );
    assert_eq!(old_gap.missing_ledgers, 1);
    assert!(!old_gap.building_validated_tip);

    runtime
        .complete_validated_tip
        .store(false, Ordering::Relaxed);
    let building_tip = policy.evaluate_with_progress(&runtime, 900, 1_156);
    assert_eq!(building_tip.missing_ledgers, 1);
    assert!(building_tip.building_validated_tip);
    assert_eq!(
        building_tip.status,
        app::SHAMapStoreHealthStatus::Waiting(Duration::from_secs(1))
    );

    runtime.missing_ledgers.store(0, Ordering::Relaxed);
    runtime
        .complete_validated_tip
        .store(true, Ordering::Relaxed);
    assert_eq!(
        policy.evaluate_with_progress(&runtime, 900, 1_156).status,
        app::SHAMapStoreHealthStatus::KeepGoing,
        "filling the older gap resumes a safe rotation attempt"
    );
}

#[test]
fn shamap_store_worker_abandons_a_gap_introduced_after_copy_before_next_stage() {
    let mut store = SHAMapStore::new(256, true, 0);
    store.set_last_rotated(900);
    store.set_can_delete(900);
    store.set_max_waiting_ledgers(2);
    store.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_156, 0, false,
    )));
    let mut runtime = healthy_runtime();
    runtime.validated_seq.store(1_156, Ordering::Relaxed);
    runtime.introduce_gap_after_copy = true;
    runtime.advance_on_sleep = true;

    let step = run_shamap_store_worker_step(&mut store, &mut runtime, None)
        .expect("worker step")
        .expect("queued ledger");

    assert!(!step.rotated);
    assert!(
        !step.stopped,
        "a gap abandons rather than stops the service"
    );
    assert_eq!(store.get_last_rotated(), 900);
    assert_eq!(
        runtime.events,
        vec![
            "clear_prior:900".to_owned(),
            "copy:1156".to_owned(),
            "sleep:2s".to_owned(),
            "sleep:2s".to_owned(),
        ],
        "the post-copy health checkpoint must prevent cache freshening, backend preparation, and rotation"
    );
}

#[test]
fn shamap_store_worker_circuit_breaker_abandons_without_advancing_boundary() {
    let mut store = SHAMapStore::new(256, true, 0);
    store.set_last_rotated(900);
    store.set_can_delete(900);
    store.set_max_waiting_ledgers(2);
    store.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_156, 0, false,
    )));
    let mut runtime = healthy_runtime();
    runtime.mode = SHAMapStoreOperatingMode::Other;
    runtime.validated_seq.store(1_156, Ordering::Relaxed);
    runtime.advance_on_sleep = true;

    let step = run_shamap_store_worker_step(&mut store, &mut runtime, None)
        .expect("worker step")
        .expect("queued ledger");

    assert!(!step.rotated);
    assert!(!step.stopped);
    assert_eq!(store.get_last_rotated(), 900);
    assert_eq!(store.saved_state().last_rotated, 900);
    assert_eq!(
        runtime.events,
        vec!["sleep:2s".to_owned(), "sleep:2s".to_owned()],
        "recovery waits are bounded by validated-ledger progress and never spin"
    );
}

#[test]
fn shamap_store_health_uses_rippled_disconnected_maintenance_window() {
    let mut runtime = healthy_runtime();
    runtime.mode = SHAMapStoreOperatingMode::Other;
    runtime.disconnected = true;
    runtime.validated_seq.store(1_156, Ordering::Relaxed);
    let policy = SHAMapStoreHealthPolicy {
        age_threshold: Duration::ZERO,
        recovery_wait: Duration::from_secs(2),
    };

    assert_eq!(
        policy.evaluate_with_progress(&runtime, 900, 1_156).status,
        app::SHAMapStoreHealthStatus::KeepGoing
    );
}

#[test]
fn shamap_store_worker_restart_uses_rippled_process_local_health_anchor() {
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
    runtime.validated_seq.store(1_156, Ordering::Relaxed);
    runtime.missing_before_seq.store(900, Ordering::Relaxed);

    let step = run_shamap_store_worker_step(&mut store, &mut runtime, None)
        .expect("worker step")
        .expect("queued ledger");

    assert!(step.rotated);
    assert!(!step.stopped);
    assert_eq!(store.get_last_rotated(), 1_156);
    assert_eq!(store.saved_state().last_rotated, 1_156);
    assert!(runtime.events.iter().any(|event| event == "rotate"));
    assert!(
        !runtime
            .events
            .iter()
            .any(|event| event.starts_with("sleep:")),
        "rippled does not seed its process-local health range from the durable rotation boundary"
    );
}

#[test]
fn shamap_store_worker_rotates_while_disconnected_like_rippled() {
    let initial = SHAMapStoreSavedState {
        writable_db: "writable.current".to_owned(),
        archive_db: "archive.current".to_owned(),
        last_rotated: 900,
    };
    let (_directory, state_db) = durable_state_db(&initial);
    let mut store = SHAMapStore::new(256, true, 0);
    store.set_saved_state(initial.clone());
    store.set_can_delete(900);
    store.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_156, 0, false,
    )));
    let mut runtime = healthy_runtime();
    runtime.disconnected = true;

    let step = run_shamap_store_worker_step(&mut store, &mut runtime, Some(&state_db))
        .expect("worker step")
        .expect("queued ledger");

    assert!(step.rotated);
    assert!(!step.stopped);
    assert_eq!(store.get_last_rotated(), 1_156);
    assert!(runtime.events.iter().any(|event| event == "rotate"));
    assert_eq!(
        state_db.get_state().expect("durable state").last_rotated,
        1_156
    );
}

#[test]
fn shamap_store_worker_continues_mid_rotation_disconnect_like_rippled() {
    let initial = SHAMapStoreSavedState {
        writable_db: "writable.current".to_owned(),
        archive_db: "archive.current".to_owned(),
        last_rotated: 900,
    };
    let (_directory, state_db) = durable_state_db(&initial);
    let mut store = SHAMapStore::new(256, true, 0);
    store.set_saved_state(initial.clone());
    store.set_can_delete(900);
    store.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_156, 0, false,
    )));
    let mut runtime = healthy_runtime();
    runtime.disconnect_after_copy = true;

    let completed = run_shamap_store_worker_step(&mut store, &mut runtime, Some(&state_db))
        .expect("worker step")
        .expect("queued ledger");

    assert!(completed.rotated);
    assert!(!completed.stopped);
    assert_eq!(store.get_last_rotated(), 1_156);
    assert_eq!(
        state_db.get_state().expect("durable state").last_rotated,
        1_156
    );
    assert!(runtime.events.iter().any(|event| event == "rotate"));
}

#[test]
fn shamap_store_worker_abandons_gap_immediately_before_cache_clear_and_retries() {
    let initial = SHAMapStoreSavedState {
        writable_db: "writable.current".to_owned(),
        archive_db: "archive.current".to_owned(),
        last_rotated: 900,
    };
    let (_directory, state_db) = durable_state_db(&initial);
    let mut store = SHAMapStore::new(256, true, 0);
    store.set_saved_state(initial.clone());
    store.set_can_delete(900);
    store.set_max_waiting_ledgers(2);
    store.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_156, 0, false,
    )));
    let mut runtime = healthy_runtime();
    runtime.validated_seq.store(1_156, Ordering::Relaxed);
    runtime.advance_on_sleep = true;
    runtime.introduce_gap_before_clear_caches = true;

    let abandoned = run_shamap_store_worker_step(&mut store, &mut runtime, Some(&state_db))
        .expect("worker step")
        .expect("queued ledger");

    assert!(!abandoned.rotated);
    assert!(!abandoned.stopped);
    assert_eq!(store.get_last_rotated(), 900);
    assert_eq!(store.saved_state(), &initial);
    assert_eq!(state_db.get_state().expect("durable state"), initial);
    assert_eq!(
        runtime.events,
        vec![
            "clear_prior:900".to_owned(),
            "copy:1156".to_owned(),
            "freshen".to_owned(),
            "prepare_rotation".to_owned(),
            "sleep:2s".to_owned(),
            "sleep:2s".to_owned(),
        ],
        "the checkpoint immediately before clear_caches must stop cache eviction and rotation"
    );

    runtime.introduce_gap_before_clear_caches = false;
    runtime.missing_ledgers.store(0, Ordering::Relaxed);
    runtime.advance_on_sleep = false;
    store.set_max_waiting_ledgers(256);
    store.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_156, 0, false,
    )));
    let retried = run_shamap_store_worker_step(&mut store, &mut runtime, Some(&state_db))
        .expect("retry worker step")
        .expect("retry queued ledger");

    assert!(retried.rotated);
    assert_eq!(store.get_last_rotated(), 1_156);
    assert_eq!(
        state_db
            .get_state()
            .expect("retried durable state")
            .last_rotated,
        1_156
    );
}
