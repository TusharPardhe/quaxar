use crate::SHAMapStoreSavedState;
use ledger::Ledger;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct SHAMapStoreRuntimeState {
    pub can_delete: u32,
    pub minimum_online: u32,
    pub working: bool,
    pub stop: bool,
    pub healthy: bool,
    /// Highest validated sequence whose complete-ledger range passed the
    /// online-delete health check. A gap below this value remains blocking
    /// until it is repaired; it must not be advanced by a failed check.
    pub last_good_validated_ledger: u32,
    /// Latest validated sequence at which the health check succeeded. This
    /// anchors the bounded recovery circuit breaker for one rotation attempt.
    pub last_successful_health_check: u32,
    pub queued_ledger: Option<Arc<Ledger>>,
    pub saved_state: SHAMapStoreSavedState,
}

impl Default for SHAMapStoreRuntimeState {
    fn default() -> Self {
        Self {
            can_delete: u32::MAX,
            minimum_online: 0,
            working: false,
            stop: false,
            healthy: true,
            last_good_validated_ledger: 0,
            last_successful_health_check: 0,
            queued_ledger: None,
            saved_state: SHAMapStoreSavedState::default(),
        }
    }
}

impl SHAMapStoreRuntimeState {
    pub fn on_ledger_closed(&mut self, ledger: Arc<Ledger>) {
        self.queued_ledger = Some(ledger);
        self.working = true;
    }

    pub fn finish_rendezvous(&mut self) {
        self.working = false;
    }

    pub fn set_stop(&mut self, stop: bool) {
        self.stop = stop;
    }

    pub fn note_rotation_boundary(&mut self, last_rotated: u32) {
        self.minimum_online = last_rotated.saturating_add(1);
    }

    pub fn set_last_rotated(&mut self, last_rotated: u32) {
        self.saved_state.last_rotated = last_rotated;
    }
}
