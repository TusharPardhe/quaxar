use nodestore::snapshot::SnapshotExportCancellation;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotExportPhase {
    Idle,
    Running,
    Completed,
    Failed,
}

impl SnapshotExportPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotExportStatus {
    pub phase: SnapshotExportPhase,
    pub output: Option<String>,
    pub ledger_seq: Option<u32>,
    pub file_size: Option<u64>,
    pub error: Option<String>,
}

impl Default for SnapshotExportStatus {
    fn default() -> Self {
        Self {
            phase: SnapshotExportPhase::Idle,
            output: None,
            ledger_seq: None,
            file_size: None,
            error: None,
        }
    }
}

#[derive(Default)]
struct SnapshotExportLifecycle {
    stopping: bool,
    cancellation: Option<SnapshotExportCancellation>,
    worker: Option<JoinHandle<()>>,
}

#[derive(Default)]
pub struct SnapshotExportState {
    status: Mutex<SnapshotExportStatus>,
    lifecycle: Mutex<SnapshotExportLifecycle>,
}

impl SnapshotExportState {
    fn begin(&self, output: String, ledger_seq: u32) -> Result<(), String> {
        let mut status = self
            .status
            .lock()
            .expect("snapshot export state mutex must not be poisoned");
        if status.phase == SnapshotExportPhase::Running {
            let output = status.output.as_deref().unwrap_or("an unknown path");
            return Err(format!("Snapshot export is already running to {output}"));
        }
        *status = SnapshotExportStatus {
            phase: SnapshotExportPhase::Running,
            output: Some(output),
            ledger_seq: Some(ledger_seq),
            file_size: None,
            error: None,
        };
        Ok(())
    }

    /// Start an export while atomically registering its worker with the
    /// application owner. Shutdown takes the same lifecycle lock before it
    /// takes the worker to join, so NodeStore teardown cannot race an RPC
    /// export that has been admitted but not yet recorded.
    pub fn start(
        self: &Arc<Self>,
        output: String,
        ledger_seq: u32,
        spawn: impl FnOnce(Arc<Self>, SnapshotExportCancellation) -> Result<JoinHandle<()>, String>,
    ) -> Result<(), String> {
        let stale_worker = {
            let mut lifecycle = self
                .lifecycle
                .lock()
                .expect("snapshot export lifecycle mutex must not be poisoned");
            if lifecycle.stopping {
                return Err("Snapshot export is unavailable while shutting down".to_owned());
            }
            if lifecycle
                .worker
                .as_ref()
                .is_some_and(JoinHandle::is_finished)
            {
                lifecycle.cancellation.take();
                lifecycle.worker.take()
            } else {
                None
            }
        };
        if let Some(worker) = stale_worker
            && worker.join().is_err()
        {
            self.fail("Snapshot export worker panicked".to_owned());
        }

        let mut lifecycle = self
            .lifecycle
            .lock()
            .expect("snapshot export lifecycle mutex must not be poisoned");
        if lifecycle.stopping {
            return Err("Snapshot export is unavailable while shutting down".to_owned());
        }
        self.begin(output, ledger_seq)?;
        let cancellation = SnapshotExportCancellation::new();
        match spawn(Arc::clone(self), cancellation.clone()) {
            Ok(worker) => {
                lifecycle.cancellation = Some(cancellation);
                lifecycle.worker = Some(worker);
                Ok(())
            }
            Err(error) => {
                self.fail(error.clone());
                Err(error)
            }
        }
    }

    /// Prevent further exports, request cooperative cancellation, and join
    /// the active worker before its backing storage is stopped.
    pub fn quiesce(&self) {
        let (cancellation, worker) = {
            let mut lifecycle = self
                .lifecycle
                .lock()
                .expect("snapshot export lifecycle mutex must not be poisoned");
            lifecycle.stopping = true;
            (lifecycle.cancellation.take(), lifecycle.worker.take())
        };
        if let Some(cancellation) = cancellation {
            cancellation.cancel();
        }
        if let Some(worker) = worker
            && worker.join().is_err()
        {
            self.fail("Snapshot export worker panicked during shutdown".to_owned());
        }
    }

    pub fn complete(&self, file_size: u64) {
        let mut status = self
            .status
            .lock()
            .expect("snapshot export state mutex must not be poisoned");
        status.phase = SnapshotExportPhase::Completed;
        status.file_size = Some(file_size);
        status.error = None;
    }

    pub fn fail(&self, error: String) {
        let mut status = self
            .status
            .lock()
            .expect("snapshot export state mutex must not be poisoned");
        status.phase = SnapshotExportPhase::Failed;
        status.file_size = None;
        status.error = Some(error);
    }

    pub fn snapshot(&self) -> SnapshotExportStatus {
        self.status
            .lock()
            .expect("snapshot export state mutex must not be poisoned")
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{SnapshotExportPhase, SnapshotExportState};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn snapshot_export_state_tracks_terminal_outcomes_and_rejects_overlap() {
        let state = Arc::new(SnapshotExportState::default());
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        state
            .start(
                "/tmp/test.xrpls".to_owned(),
                123,
                move |state, _cancellation| {
                    std::thread::Builder::new()
                        .name("snapshot-state-test".to_owned())
                        .spawn(move || {
                            started_tx.send(()).expect("signal export start");
                            finish_rx.recv().expect("release export");
                            state.complete(456);
                        })
                        .map_err(|error| error.to_string())
                },
            )
            .expect("first export should start");
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first export worker should start");
        assert!(
            state
                .start("/tmp/second.xrpls".to_owned(), 124, |_, _| {
                    unreachable!("overlapping export must not spawn")
                })
                .is_err()
        );
        finish_tx.send(()).expect("release export worker");
        state.quiesce();

        let completed = state.snapshot();
        assert_eq!(completed.phase, SnapshotExportPhase::Completed);
        assert_eq!(completed.file_size, Some(456));
        assert_eq!(completed.error, None);
    }

    #[test]
    fn quiesce_joins_the_owned_export_and_rejects_new_work() {
        let state = Arc::new(SnapshotExportState::default());
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (cancelled_tx, cancelled_rx) = std::sync::mpsc::channel();
        state
            .start(
                "/tmp/join.xrpls".to_owned(),
                321,
                move |state, cancellation| {
                    std::thread::Builder::new()
                        .name("snapshot-state-join-test".to_owned())
                        .spawn(move || {
                            started_tx.send(()).expect("signal export start");
                            while !cancellation.is_cancelled() {
                                std::thread::yield_now();
                            }
                            cancelled_tx.send(()).expect("signal cancellation");
                            state.fail("snapshot export cancelled".to_owned());
                        })
                        .map_err(|error| error.to_string())
                },
            )
            .expect("export should start");
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("export worker should start");

        let waiting_state = Arc::clone(&state);
        let shutdown = std::thread::spawn(move || waiting_state.quiesce());
        cancelled_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("shutdown must request cancellation before joining");
        shutdown.join().expect("quiesce should join worker");

        let cancelled = state.snapshot();
        assert_eq!(cancelled.phase, SnapshotExportPhase::Failed);
        assert_eq!(
            cancelled.error.as_deref(),
            Some("snapshot export cancelled")
        );
        assert!(
            state
                .start("/tmp/after-shutdown.xrpls".to_owned(), 322, |_, _| {
                    unreachable!("shutdown must reject a new export")
                })
                .is_err()
        );
    }
}
