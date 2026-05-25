use crate::{
    SHAMapStoreConfig, SHAMapStoreHealthStatus, SHAMapStoreRotationDecision,
    SHAMapStoreRuntimeState, SHAMapStoreSavedState, initialize_last_rotated, rotation_ready,
};
use basics::basic_config::BasicConfig;
use ledger::Ledger;
use std::sync::Arc;

pub trait SHAMapStoreRuntime {
    fn start_background_work(&mut self);
    fn stop_background_work(&mut self);
    fn minimum_sql_seq(&self) -> Option<u32>;
}

#[derive(Debug, Clone)]
pub struct SHAMapStore {
    config: SHAMapStoreConfig,
    fd_required: i32,
    runtime: SHAMapStoreRuntimeState,
}

impl Default for SHAMapStore {
    fn default() -> Self {
        Self::new(0, false, i32::default())
    }
}

impl SHAMapStore {
    pub fn new(delete_interval: u32, advisory_delete: bool, fd_required: i32) -> Self {
        Self {
            config: SHAMapStoreConfig {
                delete_interval,
                advisory_delete,
                ..SHAMapStoreConfig::default()
            },
            fd_required,
            runtime: SHAMapStoreRuntimeState::default(),
        }
    }

    pub fn from_config(
        config: &BasicConfig,
        standalone: bool,
        ledger_history: u32,
        fd_required: i32,
    ) -> Result<Self, String> {
        Ok(Self {
            config: SHAMapStoreConfig::from_config(config, standalone, ledger_history)?,
            fd_required,
            runtime: SHAMapStoreRuntimeState::default(),
        })
    }

    pub const fn delete_interval(&self) -> u32 {
        self.config.delete_interval
    }

    pub fn set_delete_interval(&mut self, delete_interval: u32) {
        self.config.delete_interval = delete_interval;
    }

    pub const fn advisory_delete(&self) -> bool {
        self.config.advisory_delete
    }

    pub fn set_advisory_delete(&mut self, advisory_delete: bool) {
        self.config.advisory_delete = advisory_delete;
    }

    pub const fn clamp_fetch_depth(&self, fetch_depth: u32) -> u32 {
        if self.config.delete_interval != 0 && fetch_depth > self.config.delete_interval {
            self.config.delete_interval
        } else {
            fetch_depth
        }
    }

    pub fn on_ledger_closed(&mut self, ledger: Arc<Ledger>) {
        self.runtime.on_ledger_closed(ledger);
    }

    pub fn start<R>(&mut self, runtime: &mut R) -> bool
    where
        R: SHAMapStoreRuntime + ?Sized,
    {
        if self.config.delete_interval == 0 {
            return false;
        }

        self.runtime.set_stop(false);
        runtime.start_background_work();
        true
    }

    pub fn rendezvous(&self) -> bool {
        !self.runtime.working
    }

    pub fn finish_rendezvous(&mut self) {
        self.runtime.finish_rendezvous();
    }

    pub fn stop<R>(&mut self, runtime: &mut R) -> bool
    where
        R: SHAMapStoreRuntime + ?Sized,
    {
        if self.config.delete_interval == 0 {
            return false;
        }

        self.runtime.set_stop(true);
        runtime.stop_background_work();
        true
    }

    pub fn request_stop(&mut self) -> bool {
        if self.config.delete_interval == 0 {
            return false;
        }

        self.runtime.set_stop(true);
        true
    }

    pub const fn is_stopping(&self) -> bool {
        self.runtime.stop
    }

    pub fn set_can_delete(&mut self, can_delete: u32) -> u32 {
        if self.config.advisory_delete {
            self.runtime.can_delete = can_delete;
        }
        can_delete
    }

    pub const fn get_last_rotated(&self) -> u32 {
        self.runtime.saved_state.last_rotated
    }

    pub fn set_last_rotated(&mut self, ledger_seq: u32) {
        self.runtime.set_last_rotated(ledger_seq);
    }

    pub const fn get_can_delete(&self) -> u32 {
        self.runtime.can_delete
    }

    pub const fn fd_required(&self) -> i32 {
        self.fd_required
    }

    pub fn set_fd_required(&mut self, fd_required: i32) {
        self.fd_required = fd_required;
    }

    pub fn queued_ledger_seq(&self) -> Option<u32> {
        self.runtime
            .queued_ledger
            .as_ref()
            .map(|ledger| ledger.header().seq)
    }

    pub fn take_queued_ledger(&mut self) -> Option<Arc<Ledger>> {
        self.runtime.queued_ledger.take()
    }

    pub fn note_rotation_boundary(&mut self, last_rotated: u32) {
        self.runtime.note_rotation_boundary(last_rotated);
    }

    pub fn minimum_online<R>(&self, runtime: &R) -> Option<u32>
    where
        R: SHAMapStoreRuntime + ?Sized,
    {
        if self.config.delete_interval != 0 && self.runtime.minimum_online != 0 {
            Some(self.runtime.minimum_online)
        } else {
            runtime.minimum_sql_seq()
        }
    }

    pub fn saved_state(&self) -> &SHAMapStoreSavedState {
        &self.runtime.saved_state
    }

    pub fn saved_state_mut(&mut self) -> &mut SHAMapStoreSavedState {
        &mut self.runtime.saved_state
    }

    pub fn set_saved_state(&mut self, state: SHAMapStoreSavedState) {
        self.runtime.saved_state = state;
    }

    pub fn initialize_last_rotated(&mut self, validated_seq: u32) -> u32 {
        let last_rotated =
            initialize_last_rotated(self.runtime.saved_state.last_rotated, validated_seq);
        self.runtime.saved_state.last_rotated = last_rotated;
        last_rotated
    }

    pub fn rotation_decision(
        &self,
        validated_seq: u32,
        health: SHAMapStoreHealthStatus,
    ) -> SHAMapStoreRotationDecision {
        let last_rotated =
            initialize_last_rotated(self.runtime.saved_state.last_rotated, validated_seq);
        SHAMapStoreRotationDecision {
            last_rotated,
            ready_to_rotate: rotation_ready(
                validated_seq,
                last_rotated,
                self.config.delete_interval,
                self.runtime.can_delete,
                health,
            ),
        }
    }

    pub fn config(&self) -> &SHAMapStoreConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::{SHAMapStore, SHAMapStoreRuntime};
    use ledger::Ledger;
    use std::sync::Arc;

    #[derive(Debug, Default)]
    struct TestRuntime {
        starts: usize,
        stops: usize,
        minimum_sql_seq: Option<u32>,
    }

    impl SHAMapStoreRuntime for TestRuntime {
        fn start_background_work(&mut self) {
            self.starts += 1;
        }

        fn stop_background_work(&mut self) {
            self.stops += 1;
        }

        fn minimum_sql_seq(&self) -> Option<u32> {
            self.minimum_sql_seq
        }
    }

    #[test]
    fn shamap_store_clamps_fetch_depth_only_when_online_delete_is_enabled() {
        let disabled = SHAMapStore::new(0, false, 0);
        let enabled = SHAMapStore::new(256, false, 0);

        assert_eq!(disabled.clamp_fetch_depth(300), 300);
        assert_eq!(enabled.clamp_fetch_depth(300), 256);
        assert_eq!(enabled.clamp_fetch_depth(200), 200);
    }

    #[test]
    fn shamap_store_start_and_stop_only_run_when_delete_interval_is_nonzero() {
        let mut runtime = TestRuntime::default();
        let mut disabled = SHAMapStore::new(0, false, 0);
        let mut enabled = SHAMapStore::new(256, false, 0);

        assert!(!disabled.start(&mut runtime));
        assert!(enabled.start(&mut runtime));
        assert!(!disabled.stop(&mut runtime));
        assert!(enabled.stop(&mut runtime));

        assert_eq!(runtime.starts, 1);
        assert_eq!(runtime.stops, 1);
        assert!(enabled.is_stopping());
    }

    #[test]
    fn shamap_store_tracks_queued_ledger_and_rendezvous_state_owner() {
        let mut store = SHAMapStore::new(256, false, 0);
        let ledger = Arc::new(Ledger::from_ledger_seq_and_close_time(900, 0, false));

        assert!(store.rendezvous());
        store.on_ledger_closed(Arc::clone(&ledger));
        assert!(!store.rendezvous());
        assert_eq!(store.queued_ledger_seq(), Some(900));
        assert_eq!(
            store
                .take_queued_ledger()
                .expect("queued ledger should be present")
                .header()
                .seq,
            ledger.header().seq
        );
        assert_eq!(store.queued_ledger_seq(), None);

        store.finish_rendezvous();
        assert!(store.rendezvous());
    }

    #[test]
    fn shamap_store_advisory_can_delete_rule() {
        let mut passive = SHAMapStore::new(256, false, 0);
        let mut advisory = SHAMapStore::new(256, true, 0);

        assert_eq!(passive.get_can_delete(), u32::MAX);
        assert_eq!(passive.set_can_delete(600), 600);
        assert_eq!(passive.get_can_delete(), u32::MAX);

        assert_eq!(advisory.set_can_delete(600), 600);
        assert_eq!(advisory.get_can_delete(), 600);
    }

    #[test]
    fn shamap_store_prefers_rotation_boundary_for_minimum_online() {
        let mut runtime = TestRuntime {
            minimum_sql_seq: Some(700),
            ..TestRuntime::default()
        };
        let mut store = SHAMapStore::new(256, false, 0);
        assert_eq!(store.minimum_online(&runtime), Some(700));

        store.note_rotation_boundary(900);
        assert_eq!(store.minimum_online(&runtime), Some(901));

        runtime.minimum_sql_seq = Some(1_000);
        assert_eq!(store.minimum_online(&runtime), Some(901));
    }
}
