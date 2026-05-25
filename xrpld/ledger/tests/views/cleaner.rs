use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use ledger::{
    LedgerCleaner, LedgerCleanerJournal, LedgerCleanerLoopAction, LedgerCleanerRangeProvider,
    LedgerCleanerRequest, LedgerCleanerRuntime, LedgerCleanerState, LedgerCleanerStatus,
    configure_ledger_cleaner, ledger_cleaner_status, note_ledger_cleaner_failure,
    note_ledger_cleaner_success, plan_ledger_cleaner_iteration,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
struct StaticRangeProvider {
    min: u32,
    max: u32,
}

impl LedgerCleanerRangeProvider for StaticRangeProvider {
    fn full_validated_range(&self) -> (u32, u32) {
        (self.min, self.max)
    }
}

#[derive(Default)]
struct RecordingJournal {
    debug: Mutex<Vec<String>>,
    info: Mutex<Vec<String>>,
}

impl RecordingJournal {
    fn has_debug(&self, needle: &str) -> bool {
        self.debug
            .lock()
            .expect("debug journal poisoned")
            .iter()
            .any(|message| message.contains(needle))
    }

    fn has_info(&self, needle: &str) -> bool {
        self.info
            .lock()
            .expect("info journal poisoned")
            .iter()
            .any(|message| message.contains(needle))
    }
}

impl LedgerCleanerJournal for RecordingJournal {
    fn debug(&self, message: &str) {
        self.debug
            .lock()
            .expect("debug journal poisoned")
            .push(message.to_owned());
    }

    fn info(&self, message: &str) {
        self.info
            .lock()
            .expect("info journal poisoned")
            .push(message.to_owned());
    }
}

#[derive(Default)]
struct TestRuntime {
    loaded_local: Mutex<bool>,
    hashes: Mutex<VecDeque<Option<SHAMapHash>>>,
    results: Mutex<VecDeque<bool>>,
    calls: Mutex<Vec<(u32, SHAMapHash, bool, bool)>>,
    wait_for_load_calls: Mutex<u32>,
    success_backoff_calls: Mutex<u32>,
    failure_backoff_calls: Mutex<u32>,
}

impl TestRuntime {
    fn set_loaded_local(&self, value: bool) {
        *self
            .loaded_local
            .lock()
            .expect("loaded-local mutex poisoned") = value;
    }

    fn enqueue_results(&self, results: impl IntoIterator<Item = bool>) {
        self.results
            .lock()
            .expect("results mutex poisoned")
            .extend(results);
    }

    fn enqueue_hashes(&self, hashes: impl IntoIterator<Item = Option<SHAMapHash>>) {
        self.hashes
            .lock()
            .expect("hashes mutex poisoned")
            .extend(hashes);
    }
}

impl LedgerCleanerRuntime for TestRuntime {
    fn is_loaded_local(&self) -> bool {
        *self
            .loaded_local
            .lock()
            .expect("loaded-local mutex poisoned")
    }

    fn get_ledger_hash(&self, _ledger_index: u32) -> Option<SHAMapHash> {
        self.hashes
            .lock()
            .expect("hashes mutex poisoned")
            .pop_front()
            .unwrap_or_else(|| Some(SHAMapHash::new(Uint256::from_array([0xAB; 32]))))
    }

    fn process_ledger(
        &self,
        ledger_index: u32,
        ledger_hash: SHAMapHash,
        check_nodes: bool,
        fix_txns: bool,
    ) -> bool {
        self.calls.lock().expect("calls mutex poisoned").push((
            ledger_index,
            ledger_hash,
            check_nodes,
            fix_txns,
        ));
        self.results
            .lock()
            .expect("results mutex poisoned")
            .pop_front()
            .unwrap_or(true)
    }

    fn on_wait_for_load(&self) {
        *self
            .wait_for_load_calls
            .lock()
            .expect("wait-for-load mutex poisoned") += 1;
    }

    fn on_failure_backoff(&self) {
        *self
            .failure_backoff_calls
            .lock()
            .expect("failure-backoff mutex poisoned") += 1;
    }

    fn on_success_backoff(&self) {
        *self
            .success_backoff_calls
            .lock()
            .expect("success-backoff mutex poisoned") += 1;
    }
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("condition not met before timeout");
}

#[test]
fn cleaner_request_defaults_to_validated_range_and_resets_failures() {
    let state = configure_ledger_cleaner(LedgerCleanerRequest {
        validated_min: 10,
        validated_max: 20,
        ..LedgerCleanerRequest::default()
    });

    assert_eq!(
        state,
        LedgerCleanerState {
            min_ledger: 10,
            max_ledger: 20,
            check_nodes: false,
            fix_txns: false,
            failures: 0,
        }
    );
}

#[test]
fn cleaner_single_ledger_fast_path_enables_both_checks() {
    let state = configure_ledger_cleaner(LedgerCleanerRequest {
        validated_min: 1,
        validated_max: 50,
        ledger: Some(25),
        ..LedgerCleanerRequest::default()
    });

    assert_eq!(state.min_ledger, 25);
    assert_eq!(state.max_ledger, 25);
    assert!(state.check_nodes);
    assert!(state.fix_txns);
}

#[test]
fn cleaner_full_and_explicit_overrides_follow() {
    let state = configure_ledger_cleaner(LedgerCleanerRequest {
        validated_min: 1,
        validated_max: 100,
        full: Some(true),
        fix_txns: Some(false),
        check_nodes: Some(true),
        ..LedgerCleanerRequest::default()
    });

    assert!(!state.fix_txns);
    assert!(state.check_nodes);
}

#[test]
fn cleaner_stop_request_clears_ranges() {
    let state = configure_ledger_cleaner(LedgerCleanerRequest {
        validated_min: 1,
        validated_max: 100,
        stop: true,
        ..LedgerCleanerRequest::default()
    });

    assert_eq!(state.min_ledger, 0);
    assert_eq!(state.max_ledger, 0);
}

#[test]
fn cleaner_status_matches_idle_and_running_shapes() {
    assert_eq!(
        ledger_cleaner_status(LedgerCleanerState::default()),
        LedgerCleanerStatus::Idle
    );

    assert_eq!(
        ledger_cleaner_status(LedgerCleanerState {
            min_ledger: 2,
            max_ledger: 9,
            check_nodes: true,
            fix_txns: false,
            failures: 3,
        }),
        LedgerCleanerStatus::Running {
            min_ledger: 2,
            max_ledger: 9,
            check_nodes: true,
            fix_txns: false,
            failures: 3,
        }
    );
}

#[test]
fn cleaner_iteration_waits_stops_and_processes() {
    let active = LedgerCleanerState {
        min_ledger: 7,
        max_ledger: 9,
        check_nodes: true,
        fix_txns: false,
        failures: 0,
    };

    assert_eq!(
        plan_ledger_cleaner_iteration(active, true),
        LedgerCleanerLoopAction::WaitForLoad
    );
    assert_eq!(
        plan_ledger_cleaner_iteration(LedgerCleanerState::default(), false),
        LedgerCleanerLoopAction::Stop
    );
    assert_eq!(
        plan_ledger_cleaner_iteration(active, false),
        LedgerCleanerLoopAction::Process {
            ledger_index: 9,
            check_nodes: true,
            fix_txns: false,
        }
    );
}

#[test]
fn cleaner_success_and_failure_update_range_and_failure_count() {
    let state = LedgerCleanerState {
        min_ledger: 5,
        max_ledger: 5,
        check_nodes: true,
        fix_txns: true,
        failures: 2,
    };

    let failed = note_ledger_cleaner_failure(state);
    assert_eq!(failed.failures, 3);

    let succeeded = note_ledger_cleaner_success(failed, 5);
    assert_eq!(succeeded.min_ledger, 6);
    assert_eq!(succeeded.max_ledger, 4);
    assert_eq!(succeeded.failures, 0);
}

#[test]
fn cleaner_owner_runs_background_range_in_descending_order() {
    let runtime = Arc::new(TestRuntime::default());
    let journal = Arc::new(RecordingJournal::default());
    let cleaner = LedgerCleaner::new(
        Arc::new(StaticRangeProvider { min: 10, max: 12 }),
        runtime.clone(),
        journal,
    );

    cleaner.start();
    cleaner.clean(LedgerCleanerRequest {
        full: Some(true),
        ..LedgerCleanerRequest::default()
    });

    assert_eq!(
        cleaner.status(),
        LedgerCleanerStatus::Running {
            min_ledger: 10,
            max_ledger: 12,
            check_nodes: true,
            fix_txns: true,
            failures: 0,
        }
    );
    assert_eq!(cleaner.on_write()["status"], "running");

    wait_until(|| cleaner.status() == LedgerCleanerStatus::Idle);

    assert_eq!(
        *runtime.calls.lock().expect("calls mutex poisoned"),
        vec![
            (
                12,
                SHAMapHash::new(Uint256::from_array([0xAB; 32])),
                true,
                true
            ),
            (
                11,
                SHAMapHash::new(Uint256::from_array([0xAB; 32])),
                true,
                true
            ),
            (
                10,
                SHAMapHash::new(Uint256::from_array([0xAB; 32])),
                true,
                true
            ),
        ]
    );
    assert_eq!(cleaner.on_write()["status"], "idle");
    cleaner.stop();
}

#[test]
fn cleaner_owner_waits_for_load_and_retries_failures() {
    let runtime = Arc::new(TestRuntime::default());
    runtime.set_loaded_local(true);
    runtime.enqueue_results([false, true]);
    let journal = Arc::new(RecordingJournal::default());
    let cleaner = LedgerCleaner::new(
        Arc::new(StaticRangeProvider { min: 7, max: 7 }),
        runtime.clone(),
        journal.clone(),
    );

    cleaner.start();
    cleaner.clean(LedgerCleanerRequest::default());

    wait_until(|| {
        *runtime
            .wait_for_load_calls
            .lock()
            .expect("wait-for-load mutex poisoned")
            > 0
    });
    runtime.set_loaded_local(false);

    wait_until(|| cleaner.status() == LedgerCleanerStatus::Idle);

    assert_eq!(
        *runtime.calls.lock().expect("calls mutex poisoned"),
        vec![
            (
                7,
                SHAMapHash::new(Uint256::from_array([0xAB; 32])),
                false,
                false
            ),
            (
                7,
                SHAMapHash::new(Uint256::from_array([0xAB; 32])),
                false,
                false
            ),
        ]
    );
    assert_eq!(
        *runtime
            .failure_backoff_calls
            .lock()
            .expect("failure-backoff mutex poisoned"),
        1
    );
    assert_eq!(
        *runtime
            .success_backoff_calls
            .lock()
            .expect("success-backoff mutex poisoned"),
        1
    );
    assert!(journal.has_debug("Waiting for load to subside"));
    assert!(journal.has_info("Failed to process ledger 7"));
    cleaner.stop();
}

#[test]
fn cleaner_owner_retries_when_hash_lookup_fails() {
    let runtime = Arc::new(TestRuntime::default());
    runtime.enqueue_hashes([None, Some(SHAMapHash::new(Uint256::from_array([0x44; 32])))]);
    let journal = Arc::new(RecordingJournal::default());
    let cleaner = LedgerCleaner::new(
        Arc::new(StaticRangeProvider { min: 9, max: 9 }),
        runtime.clone(),
        journal.clone(),
    );

    cleaner.start();
    cleaner.clean(LedgerCleanerRequest::default());

    wait_until(|| cleaner.status() == LedgerCleanerStatus::Idle);

    assert_eq!(
        *runtime.calls.lock().expect("calls mutex poisoned"),
        vec![(
            9,
            SHAMapHash::new(Uint256::from_array([0x44; 32])),
            false,
            false
        )]
    );
    assert_eq!(
        *runtime
            .failure_backoff_calls
            .lock()
            .expect("failure-backoff mutex poisoned"),
        1
    );
    assert!(journal.has_info("Unable to get hash for ledger 9"));
    cleaner.stop();
}
