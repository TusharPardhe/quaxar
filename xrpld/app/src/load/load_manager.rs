//! `LoadManager` port for the app runtime shell.

use crate::job::job_queue::JobQueue;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadManagerTiming {
    pub tick_interval: Duration,
    pub reporting_interval: Duration,
    pub stall_fatal_log_limit: Duration,
    pub stall_logic_error_limit: Duration,
}

impl Default for LoadManagerTiming {
    fn default() -> Self {
        Self {
            tick_interval: Duration::from_secs(1),
            reporting_interval: Duration::from_secs(10),
            stall_fatal_log_limit: Duration::from_secs(90),
            stall_logic_error_limit: Duration::from_secs(600),
        }
    }
}

pub trait LoadFeeControl: Send + Sync + 'static {
    fn raise_local_fee(&self) -> bool;
    fn lower_local_fee(&self) -> bool;
}

#[derive(Debug, Default)]
pub struct NullLoadFeeControl;

impl LoadFeeControl for NullLoadFeeControl {
    fn raise_local_fee(&self) -> bool {
        false
    }

    fn lower_local_fee(&self) -> bool {
        false
    }
}

pub trait LoadManagerEvents: Send + Sync + 'static {
    fn report_fee_change(&self);
}

#[derive(Debug, Default)]
pub struct NullLoadManagerEvents;

impl LoadManagerEvents for NullLoadManagerEvents {
    fn report_fee_change(&self) {}
}

pub trait LoadManagerJournal: Send + Sync + 'static {
    fn debug(&self, message: &str);
    fn info(&self, message: &str);
    fn warn(&self, message: &str);
    fn fatal(&self, message: &str);
}

#[derive(Debug, Default)]
pub struct NullLoadManagerJournal;

impl LoadManagerJournal for NullLoadManagerJournal {
    fn debug(&self, _message: &str) {}
    fn info(&self, _message: &str) {}
    fn warn(&self, _message: &str) {}
    fn fatal(&self, _message: &str) {}
}

#[derive(Debug)]
struct LoadManagerControl {
    stop: bool,
    armed: bool,
    last_heartbeat: Instant,
    last_report_multiple: u128,
}

struct LoadManagerInner {
    job_queue: JobQueue,
    fee_control: Arc<dyn LoadFeeControl>,
    events: Arc<dyn LoadManagerEvents>,
    journal: Arc<dyn LoadManagerJournal>,
    timing: LoadManagerTiming,
    control: Mutex<LoadManagerControl>,
    condvar: Condvar,
    thread: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Clone)]
pub struct LoadManager {
    inner: Arc<LoadManagerInner>,
}

impl std::fmt::Debug for LoadManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let control = self
            .inner
            .control
            .lock()
            .expect("load manager control mutex must not be poisoned");
        f.debug_struct("LoadManager")
            .field("armed", &control.armed)
            .field("stopped", &control.stop)
            .field("timing", &self.inner.timing)
            .finish()
    }
}

impl LoadManager {
    pub fn new(
        job_queue: JobQueue,
        fee_control: Arc<dyn LoadFeeControl>,
        events: Arc<dyn LoadManagerEvents>,
        journal: Arc<dyn LoadManagerJournal>,
    ) -> Self {
        Self::with_timing(
            job_queue,
            fee_control,
            events,
            journal,
            LoadManagerTiming::default(),
        )
    }

    pub fn with_timing(
        job_queue: JobQueue,
        fee_control: Arc<dyn LoadFeeControl>,
        events: Arc<dyn LoadManagerEvents>,
        journal: Arc<dyn LoadManagerJournal>,
        timing: LoadManagerTiming,
    ) -> Self {
        Self {
            inner: Arc::new(LoadManagerInner {
                job_queue,
                fee_control,
                events,
                journal,
                timing,
                control: Mutex::new(LoadManagerControl {
                    stop: false,
                    armed: false,
                    last_heartbeat: Instant::now(),
                    last_report_multiple: 0,
                }),
                condvar: Condvar::new(),
                thread: Mutex::new(None),
            }),
        }
    }

    pub fn activate_stall_detector(&self) {
        let mut control = self
            .inner
            .control
            .lock()
            .expect("load manager control mutex must not be poisoned");
        control.armed = true;
        control.last_heartbeat = Instant::now();
        control.last_report_multiple = 0;
    }

    pub fn heartbeat(&self) {
        let mut control = self
            .inner
            .control
            .lock()
            .expect("load manager control mutex must not be poisoned");
        control.last_heartbeat = Instant::now();
        control.last_report_multiple = 0;
    }

    pub fn start(&self) {
        let mut thread_slot = self
            .inner
            .thread
            .lock()
            .expect("load manager thread mutex must not be poisoned");
        if thread_slot.is_some() {
            return;
        }

        self.inner.journal.debug("Starting");
        let this = self.clone();
        *thread_slot = Some(thread::spawn(move || this.run()));
    }

    pub fn stop(&self) {
        {
            let mut control = self
                .inner
                .control
                .lock()
                .expect("load manager control mutex must not be poisoned");
            control.stop = true;
            self.inner.condvar.notify_all();
        }

        let handle = self
            .inner
            .thread
            .lock()
            .expect("load manager thread mutex must not be poisoned")
            .take();
        if let Some(handle) = handle {
            self.inner.journal.debug("Stopping");
            let _ = handle.join();
        }
    }

    pub fn timing(&self) -> LoadManagerTiming {
        self.inner.timing
    }

    fn run(&self) {
        loop {
            let (stop, armed, last_heartbeat, last_report_multiple) = {
                let control = self
                    .inner
                    .control
                    .lock()
                    .expect("load manager control mutex must not be poisoned");
                let (control, _) = self
                    .inner
                    .condvar
                    .wait_timeout(control, self.inner.timing.tick_interval)
                    .expect("load manager condvar wait must not be poisoned");
                (
                    control.stop,
                    control.armed,
                    control.last_heartbeat,
                    control.last_report_multiple,
                )
            };

            if stop {
                break;
            }

            let time_spent_stalled = Instant::now().saturating_duration_since(last_heartbeat);
            if !armed || time_spent_stalled < self.inner.timing.reporting_interval {
                continue;
            }

            let multiple =
                duration_multiple(time_spent_stalled, self.inner.timing.reporting_interval);
            if multiple <= last_report_multiple {
                continue;
            }

            {
                let mut control = self
                    .inner
                    .control
                    .lock()
                    .expect("load manager control mutex must not be poisoned");
                if multiple > control.last_report_multiple {
                    control.last_report_multiple = multiple;
                }
            }

            let seconds = time_spent_stalled.as_secs();
            if time_spent_stalled < self.inner.timing.stall_fatal_log_limit {
                self.inner
                    .journal
                    .warn(&format!("Server stalled for {seconds} seconds."));
                if self.inner.job_queue.is_overloaded() {
                    self.inner
                        .journal
                        .warn(&format!("JobQueue: {}", self.inner.job_queue.get_json(0)));
                }
            } else {
                self.inner
                    .journal
                    .fatal(&format!("Server stalled for {seconds} seconds."));
                self.inner
                    .journal
                    .fatal(&format!("JobQueue: {}", self.inner.job_queue.get_json(0)));
            }

            if time_spent_stalled >= self.inner.timing.stall_logic_error_limit {
                self.inner.journal.fatal(&format!(
                    "LogicError: Fatal server stall detected. Stalled time: {seconds}s"
                ));
                self.inner
                    .journal
                    .fatal(&format!("JobQueue: {}", self.inner.job_queue.get_json(0)));
                break;
            }
        }

        let change = if self.inner.job_queue.is_overloaded() {
            self.inner.journal.info(&format!(
                "Raising local fee (JQ overload): {}",
                self.inner.job_queue.get_json(0)
            ));
            self.inner.fee_control.raise_local_fee()
        } else {
            self.inner.fee_control.lower_local_fee()
        };

        if change {
            self.inner.events.report_fee_change();
        }
    }
}

impl Drop for LoadManager {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 1 {
            self.stop();
        }
    }
}

fn duration_multiple(duration: Duration, interval: Duration) -> u128 {
    if interval.is_zero() {
        return 0;
    }
    duration.as_nanos() / interval.as_nanos()
}

#[cfg(test)]
mod tests {
    use super::{
        LoadFeeControl, LoadManager, LoadManagerEvents, LoadManagerJournal, LoadManagerTiming,
    };
    use crate::job::job_queue::JobQueue;
    use crate::job::job_types::JobType;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Default)]
    struct RecordingFees {
        raised: AtomicUsize,
        lowered: AtomicUsize,
    }

    impl LoadFeeControl for RecordingFees {
        fn raise_local_fee(&self) -> bool {
            self.raised.fetch_add(1, Ordering::Relaxed);
            true
        }

        fn lower_local_fee(&self) -> bool {
            self.lowered.fetch_add(1, Ordering::Relaxed);
            true
        }
    }

    #[derive(Default)]
    struct RecordingEvents {
        reports: AtomicUsize,
    }

    impl LoadManagerEvents for RecordingEvents {
        fn report_fee_change(&self) {
            self.reports.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[derive(Default)]
    struct RecordingJournal {
        entries: Mutex<Vec<String>>,
    }

    impl LoadManagerJournal for RecordingJournal {
        fn debug(&self, message: &str) {
            self.entries
                .lock()
                .expect("journal mutex")
                .push(message.to_owned());
        }

        fn info(&self, message: &str) {
            self.entries
                .lock()
                .expect("journal mutex")
                .push(message.to_owned());
        }

        fn warn(&self, message: &str) {
            self.entries
                .lock()
                .expect("journal mutex")
                .push(message.to_owned());
        }

        fn fatal(&self, message: &str) {
            self.entries
                .lock()
                .expect("journal mutex")
                .push(message.to_owned());
        }
    }

    #[test]
    fn load_manager_adjusts_fee_on_stop_using_queue_overload_state() {
        let queue = JobQueue::default();
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        assert!(queue.add_job(JobType::JtPack, "first", move || {
            gate_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }));
        gate_rx.recv_timeout(Duration::from_secs(5)).expect("first JtPack job should start");
        assert!(queue.add_job(JobType::JtPack, "second", || {}));
        assert!(queue.is_overloaded(), "queue must be overloaded: JtPack's limit of 1 is already held by the still-running first job");
        release_tx.send(()).unwrap();

        let fees = Arc::new(RecordingFees::default());
        let events = Arc::new(RecordingEvents::default());
        let journal = Arc::new(RecordingJournal::default());
        let manager = LoadManager::with_timing(
            queue,
            fees.clone(),
            events.clone(),
            journal,
            LoadManagerTiming {
                tick_interval: Duration::from_millis(5),
                ..LoadManagerTiming::default()
            },
        );

        manager.start();
        manager.stop();

        assert_eq!(fees.raised.load(Ordering::Relaxed), 1);
        assert_eq!(fees.lowered.load(Ordering::Relaxed), 0);
        assert_eq!(events.reports.load(Ordering::Relaxed), 1);
    }
}
