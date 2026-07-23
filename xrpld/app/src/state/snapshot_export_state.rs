use std::sync::Mutex;

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
pub struct SnapshotExportState {
    status: Mutex<SnapshotExportStatus>,
}

impl SnapshotExportState {
    pub fn begin(&self, output: String, ledger_seq: u32) -> Result<(), String> {
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

    #[test]
    fn snapshot_export_state_tracks_terminal_outcomes_and_rejects_overlap() {
        let state = SnapshotExportState::default();
        assert_eq!(state.snapshot().phase, SnapshotExportPhase::Idle);

        state
            .begin("/tmp/test.xrpls".to_owned(), 123)
            .expect("first export should start");
        assert_eq!(state.snapshot().phase, SnapshotExportPhase::Running);
        assert!(state.begin("/tmp/second.xrpls".to_owned(), 124).is_err());

        state.complete(456);
        let completed = state.snapshot();
        assert_eq!(completed.phase, SnapshotExportPhase::Completed);
        assert_eq!(completed.file_size, Some(456));
        assert_eq!(completed.error, None);

        state
            .begin("/tmp/retry.xrpls".to_owned(), 125)
            .expect("a completed export should allow a later export");
        state.fail("disk full".to_owned());
        let failed = state.snapshot();
        assert_eq!(failed.phase, SnapshotExportPhase::Failed);
        assert_eq!(failed.error.as_deref(), Some("disk full"));
    }
}
