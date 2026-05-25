//! `LedgerCleaner` request/progress state machine plus the owner/background
//! wrapper that mirrors the reference thread lifecycle above explicit Rust seams.

use basics::sha_map_hash::SHAMapHash;
use serde_json::{Map, Value, json};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub const LEDGER_CLEANER_LOAD_WAIT: Duration = Duration::from_secs(5);
pub const LEDGER_CLEANER_FAILURE_WAIT: Duration = Duration::from_secs(2);
pub const LEDGER_CLEANER_SUCCESS_WAIT: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LedgerCleanerState {
    pub min_ledger: u32,
    pub max_ledger: u32,
    pub check_nodes: bool,
    pub fix_txns: bool,
    pub failures: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LedgerCleanerRequest {
    pub validated_min: u32,
    pub validated_max: u32,
    pub ledger: Option<u32>,
    pub min_ledger: Option<u32>,
    pub max_ledger: Option<u32>,
    pub full: Option<bool>,
    pub fix_txns: Option<bool>,
    pub check_nodes: Option<bool>,
    pub stop: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerCleanerStatus {
    Idle,
    Running {
        min_ledger: u32,
        max_ledger: u32,
        check_nodes: bool,
        fix_txns: bool,
        failures: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerCleanerLoopAction {
    Stop,
    WaitForLoad,
    Process {
        ledger_index: u32,
        check_nodes: bool,
        fix_txns: bool,
    },
}

pub trait LedgerCleanerRangeProvider: Send + Sync + 'static {
    fn full_validated_range(&self) -> (u32, u32);
}

pub trait LedgerCleanerRuntime: Send + Sync + 'static {
    fn is_loaded_local(&self) -> bool;

    fn get_ledger_hash(&self, ledger_index: u32) -> Option<SHAMapHash>;

    fn process_ledger(
        &self,
        ledger_index: u32,
        ledger_hash: SHAMapHash,
        check_nodes: bool,
        fix_txns: bool,
    ) -> bool;

    fn on_wait_for_load(&self) {
        thread::sleep(LEDGER_CLEANER_LOAD_WAIT);
    }

    fn on_failure_backoff(&self) {
        thread::sleep(LEDGER_CLEANER_FAILURE_WAIT);
    }

    fn on_success_backoff(&self) {
        thread::sleep(LEDGER_CLEANER_SUCCESS_WAIT);
    }
}

pub trait LedgerCleanerJournal: Send + Sync + 'static {
    fn debug(&self, message: &str);
    fn info(&self, message: &str);
}

#[derive(Debug, Default)]
pub struct NullLedgerCleanerRangeProvider;

impl LedgerCleanerRangeProvider for NullLedgerCleanerRangeProvider {
    fn full_validated_range(&self) -> (u32, u32) {
        (0, 0)
    }
}

#[derive(Debug, Default)]
pub struct NullLedgerCleanerRuntime;

impl LedgerCleanerRuntime for NullLedgerCleanerRuntime {
    fn is_loaded_local(&self) -> bool {
        false
    }

    fn get_ledger_hash(&self, _ledger_index: u32) -> Option<SHAMapHash> {
        None
    }

    fn process_ledger(
        &self,
        _ledger_index: u32,
        _ledger_hash: SHAMapHash,
        _check_nodes: bool,
        _fix_txns: bool,
    ) -> bool {
        true
    }
}

#[derive(Debug, Default)]
pub struct NullLedgerCleanerJournal;

impl LedgerCleanerJournal for NullLedgerCleanerJournal {
    fn debug(&self, _message: &str) {}

    fn info(&self, _message: &str) {}
}

#[derive(Clone)]
pub struct LedgerCleaner {
    inner: Arc<LedgerCleanerInner>,
}

impl LedgerCleaner {
    pub fn new(
        range_provider: Arc<dyn LedgerCleanerRangeProvider>,
        runtime: Arc<dyn LedgerCleanerRuntime>,
        journal: Arc<dyn LedgerCleanerJournal>,
    ) -> Self {
        Self {
            inner: Arc::new(LedgerCleanerInner::new(range_provider, runtime, journal)),
        }
    }

    pub fn start(&self) {
        self.inner.start();
    }

    pub fn stop(&self) {
        self.inner.stop();
    }

    pub fn clean(&self, mut request: LedgerCleanerRequest) {
        let (validated_min, validated_max) = self.inner.range_provider.full_validated_range();
        request.validated_min = validated_min;
        request.validated_max = validated_max;

        let state = configure_ledger_cleaner(request);
        self.inner.with_control_mut(|control| {
            control.state = state;
            control.cleaning = true;
            self.inner.condvar.notify_all();
        });
    }

    pub fn status(&self) -> LedgerCleanerStatus {
        self.inner.with_control(|control| {
            if control.cleaning {
                ledger_cleaner_status(control.state)
            } else {
                LedgerCleanerStatus::Idle
            }
        })
    }

    pub fn on_write(&self) -> Value {
        self.inner.with_control(|control| {
            let mut map = Map::new();
            if !control.cleaning || control.state.max_ledger == 0 {
                map.insert("status".to_owned(), json!("idle"));
            } else {
                map.insert("status".to_owned(), json!("running"));
                map.insert("min_ledger".to_owned(), json!(control.state.min_ledger));
                map.insert("max_ledger".to_owned(), json!(control.state.max_ledger));
                map.insert(
                    "check_nodes".to_owned(),
                    json!(if control.state.check_nodes {
                        "true"
                    } else {
                        "false"
                    }),
                );
                map.insert(
                    "fix_txns".to_owned(),
                    json!(if control.state.fix_txns {
                        "true"
                    } else {
                        "false"
                    }),
                );
                if control.state.failures > 0 {
                    map.insert("fail_counts".to_owned(), json!(control.state.failures));
                }
            }
            Value::Object(map)
        })
    }
}

impl Drop for LedgerCleaner {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 1 {
            self.stop();
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct LedgerCleanerControl {
    state: LedgerCleanerState,
    cleaning: bool,
    should_exit: bool,
}

struct LedgerCleanerInner {
    range_provider: Arc<dyn LedgerCleanerRangeProvider>,
    runtime: Arc<dyn LedgerCleanerRuntime>,
    journal: Arc<dyn LedgerCleanerJournal>,
    control: Mutex<LedgerCleanerControl>,
    condvar: Condvar,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl LedgerCleanerInner {
    fn new(
        range_provider: Arc<dyn LedgerCleanerRangeProvider>,
        runtime: Arc<dyn LedgerCleanerRuntime>,
        journal: Arc<dyn LedgerCleanerJournal>,
    ) -> Self {
        Self {
            range_provider,
            runtime,
            journal,
            control: Mutex::new(LedgerCleanerControl {
                state: LedgerCleanerState::default(),
                cleaning: false,
                should_exit: false,
            }),
            condvar: Condvar::new(),
            thread: Mutex::new(None),
        }
    }

    fn with_control<R>(&self, f: impl FnOnce(&LedgerCleanerControl) -> R) -> R {
        let guard = self
            .control
            .lock()
            .expect("ledger cleaner control mutex must not be poisoned");
        f(&guard)
    }

    fn with_control_mut<R>(&self, f: impl FnOnce(&mut LedgerCleanerControl) -> R) -> R {
        let mut guard = self
            .control
            .lock()
            .expect("ledger cleaner control mutex must not be poisoned");
        f(&mut guard)
    }

    fn start(self: &Arc<Self>) {
        let mut thread_guard = self
            .thread
            .lock()
            .expect("ledger cleaner thread mutex must not be poisoned");
        if thread_guard.is_some() {
            return;
        }
        self.with_control_mut(|control| {
            control.should_exit = false;
        });
        let inner = Arc::clone(self);
        let handle = thread::Builder::new()
            .name("LedgerCleaner".to_owned())
            .spawn(move || inner.run())
            .expect("ledger cleaner thread should start");
        *thread_guard = Some(handle);
    }

    fn stop(&self) {
        self.journal.info("Stopping");
        self.with_control_mut(|control| {
            control.should_exit = true;
            self.condvar.notify_all();
        });

        if let Some(handle) = self
            .thread
            .lock()
            .expect("ledger cleaner thread mutex must not be poisoned")
            .take()
        {
            let _ = handle.join();
        }
    }

    fn run(self: Arc<Self>) {
        self.journal.debug("Started");

        loop {
            let state = {
                let mut control = self
                    .control
                    .lock()
                    .expect("ledger cleaner control mutex must not be poisoned");
                while !control.should_exit && !control.cleaning {
                    control = self
                        .condvar
                        .wait(control)
                        .expect("ledger cleaner condvar wait must succeed");
                }
                if control.should_exit {
                    return;
                }
                control.state
            };

            self.run_cleaning_loop(state);
        }
    }

    fn run_cleaning_loop(&self, mut state: LedgerCleanerState) {
        loop {
            if self.with_control(|control| control.should_exit) {
                return;
            }

            if self.runtime.is_loaded_local() {
                self.journal.debug("Waiting for load to subside");
                self.runtime.on_wait_for_load();
                state = self.with_control(|control| control.state);
                continue;
            }

            match plan_ledger_cleaner_iteration(state, false) {
                LedgerCleanerLoopAction::Stop => {
                    self.with_control_mut(|control| {
                        control.state = LedgerCleanerState::default();
                        control.cleaning = false;
                    });
                    return;
                }
                LedgerCleanerLoopAction::WaitForLoad => {
                    self.journal.debug("Waiting for load to subside");
                    self.runtime.on_wait_for_load();
                }
                LedgerCleanerLoopAction::Process {
                    ledger_index,
                    check_nodes,
                    fix_txns,
                } => {
                    let Some(ledger_hash) = self.runtime.get_ledger_hash(ledger_index) else {
                        self.journal
                            .info(&format!("Unable to get hash for ledger {ledger_index}"));
                        let next = note_ledger_cleaner_failure(state);
                        self.with_control_mut(|control| {
                            control.state = next;
                            control.cleaning = true;
                        });
                        self.runtime.on_failure_backoff();
                        state = next;
                        continue;
                    };
                    let success = self.runtime.process_ledger(
                        ledger_index,
                        ledger_hash,
                        check_nodes,
                        fix_txns,
                    );
                    state = if success {
                        let next = note_ledger_cleaner_success(state, ledger_index);
                        self.with_control_mut(|control| {
                            control.state = next;
                            control.cleaning = next.max_ledger != 0;
                        });
                        self.runtime.on_success_backoff();
                        next
                    } else {
                        self.journal
                            .info(&format!("Failed to process ledger {ledger_index}"));
                        let next = note_ledger_cleaner_failure(state);
                        self.with_control_mut(|control| {
                            control.state = next;
                            control.cleaning = true;
                        });
                        self.runtime.on_failure_backoff();
                        next
                    };
                }
            }
        }
    }
}

pub fn configure_ledger_cleaner(request: LedgerCleanerRequest) -> LedgerCleanerState {
    let mut state = LedgerCleanerState {
        min_ledger: request.validated_min,
        max_ledger: request.validated_max,
        check_nodes: false,
        fix_txns: false,
        failures: 0,
    };

    if let Some(ledger) = request.ledger {
        state.min_ledger = ledger;
        state.max_ledger = ledger;
        state.fix_txns = true;
        state.check_nodes = true;
    }

    if let Some(max_ledger) = request.max_ledger {
        state.max_ledger = max_ledger;
    }

    if let Some(min_ledger) = request.min_ledger {
        state.min_ledger = min_ledger;
    }

    if let Some(full) = request.full {
        state.fix_txns = full;
        state.check_nodes = full;
    }

    if let Some(fix_txns) = request.fix_txns {
        state.fix_txns = fix_txns;
    }

    if let Some(check_nodes) = request.check_nodes {
        state.check_nodes = check_nodes;
    }

    if request.stop {
        state.min_ledger = 0;
        state.max_ledger = 0;
    }

    state
}

pub fn ledger_cleaner_status(state: LedgerCleanerState) -> LedgerCleanerStatus {
    if state.max_ledger == 0 {
        LedgerCleanerStatus::Idle
    } else {
        LedgerCleanerStatus::Running {
            min_ledger: state.min_ledger,
            max_ledger: state.max_ledger,
            check_nodes: state.check_nodes,
            fix_txns: state.fix_txns,
            failures: state.failures,
        }
    }
}

pub fn plan_ledger_cleaner_iteration(
    state: LedgerCleanerState,
    loaded_local: bool,
) -> LedgerCleanerLoopAction {
    if loaded_local {
        return LedgerCleanerLoopAction::WaitForLoad;
    }

    if state.min_ledger > state.max_ledger || state.max_ledger == 0 || state.min_ledger == 0 {
        return LedgerCleanerLoopAction::Stop;
    }

    LedgerCleanerLoopAction::Process {
        ledger_index: state.max_ledger,
        check_nodes: state.check_nodes,
        fix_txns: state.fix_txns,
    }
}

pub fn note_ledger_cleaner_failure(mut state: LedgerCleanerState) -> LedgerCleanerState {
    state.failures += 1;
    state
}

pub fn note_ledger_cleaner_success(
    mut state: LedgerCleanerState,
    ledger_index: u32,
) -> LedgerCleanerState {
    if ledger_index == state.min_ledger {
        state.min_ledger += 1;
    }
    if ledger_index == state.max_ledger {
        state.max_ledger -= 1;
    }
    state.failures = 0;
    state
}
