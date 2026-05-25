use crate::runtime::main_runtime::ManagedComponent;
use crate::{
    SHAMapStoreComponent, SHAMapStoreHealthRuntime, SHAMapStoreOperatingMode,
    SharedSHAMapStoreHealthState,
};
use ledger::Ledger;
use std::sync::Arc;

pub struct SHAMapStoreService {
    component: Arc<SHAMapStoreComponent>,
    health: Arc<SharedSHAMapStoreHealthState>,
}

impl std::fmt::Debug for SHAMapStoreService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SHAMapStoreService")
            .field("component", &self.component)
            .field("health", &self.health)
            .finish()
    }
}

impl SHAMapStoreService {
    pub fn new(
        component: Arc<SHAMapStoreComponent>,
        health: Arc<SharedSHAMapStoreHealthState>,
    ) -> Self {
        Self { component, health }
    }

    pub fn component(&self) -> Arc<SHAMapStoreComponent> {
        Arc::clone(&self.component)
    }

    pub fn health(&self) -> Arc<SharedSHAMapStoreHealthState> {
        Arc::clone(&self.health)
    }

    pub fn on_ledger_closed(&self, ledger: Arc<Ledger>) {
        self.health.note_validated_ledger(Arc::clone(&ledger));
        self.component.on_ledger_closed(ledger);
    }

    pub fn set_operating_mode(&self, operating_mode: SHAMapStoreOperatingMode) {
        self.health.set_operating_mode(operating_mode);
    }

    pub fn set_stopping(&self, stopping: bool) {
        self.health.set_stopping(stopping);
    }

    pub fn operating_mode(&self) -> SHAMapStoreOperatingMode {
        self.health.operating_mode()
    }

    pub fn is_stopping(&self) -> bool {
        self.health.is_stopping()
    }

    pub fn validated_ledger(&self) -> Option<Arc<Ledger>> {
        self.health.validated_ledger()
    }

    pub fn validated_ledger_seq(&self) -> Option<u32> {
        self.health.validated_ledger_seq()
    }
}

impl ManagedComponent for SHAMapStoreService {
    fn start(&self) -> Result<(), String> {
        self.health.set_stopping(false);
        self.component.start()
    }

    fn stop(&self) {
        self.health.set_stopping(true);
        self.component.stop();
    }

    fn fd_required(&self) -> usize {
        self.component.fd_required()
    }
}

#[cfg(test)]
mod tests {
    use super::SHAMapStoreService;
    use crate::{
        SHAMapStore, SHAMapStoreComponent, SHAMapStoreComponentRuntime, SHAMapStoreHealthRuntime,
        SHAMapStoreOperatingMode, SHAMapStoreRuntime, SharedSHAMapStoreHealthState,
    };
    use ledger::Ledger;
    use std::sync::Arc;
    use std::time::Duration;

    #[derive(Default)]
    struct FixedCloseTimeProvider;

    impl crate::SHAMapStoreCloseTimeProvider for FixedCloseTimeProvider {
        fn current_close_time(&self) -> u32 {
            120
        }
    }

    #[derive(Default)]
    struct Runtime;

    impl SHAMapStoreRuntime for Runtime {
        fn start_background_work(&mut self) {}

        fn stop_background_work(&mut self) {}

        fn minimum_sql_seq(&self) -> Option<u32> {
            None
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

    #[test]
    fn service_updates_shared_health_and_forwards_ledgers_into_component_queue() {
        let health = Arc::new(SharedSHAMapStoreHealthState::new(Arc::new(
            FixedCloseTimeProvider,
        )));
        let component = Arc::new(SHAMapStoreComponent::new(
            SHAMapStore::new(256, false, 7),
            Box::new(Runtime),
            None,
        ));
        let service = SHAMapStoreService::new(component.clone(), health.clone());

        service.set_operating_mode(SHAMapStoreOperatingMode::Full);
        service.on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
            1_156, 100, false,
        )));

        assert_eq!(health.operating_mode(), SHAMapStoreOperatingMode::Full);
        assert_eq!(service.operating_mode(), SHAMapStoreOperatingMode::Full);
        assert_eq!(health.validated_ledger_age(), Duration::from_secs(20));
        assert_eq!(service.validated_ledger_seq(), Some(1_156));
        assert_eq!(component.snapshot().queued_ledger_seq(), Some(1_156));
    }
}
