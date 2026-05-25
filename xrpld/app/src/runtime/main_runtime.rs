//! Runtime lifecycle owner above the app root and bound managers.

use crate::state::application_root::ApplicationRoot;
use crate::tx_queue::transaction::Transaction;
use crate::tx_queue::transaction_master::{SharedTransaction, TransactionMaster};
use protocol::{JsonOptions, JsonValue};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

pub trait ManagedComponent: Send + Sync + 'static {
    fn start(&self) -> Result<(), String>;
    fn stop(&self);
    fn fd_required(&self) -> usize {
        0
    }
}

pub type ManagedHandle = Arc<dyn ManagedComponent>;

#[derive(Clone)]
pub enum GrpcRuntime {
    DisabledExplicit { reason: String },
    Enabled(ManagedHandle),
}

impl std::fmt::Debug for GrpcRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DisabledExplicit { reason } => f
                .debug_struct("GrpcRuntime")
                .field("disabled", &true)
                .field("reason", reason)
                .finish(),
            Self::Enabled(_) => f
                .debug_struct("GrpcRuntime")
                .field("disabled", &false)
                .finish(),
        }
    }
}

impl Default for GrpcRuntime {
    fn default() -> Self {
        Self::DisabledExplicit {
            reason: "gRPC is disabled until a server component is bound".to_owned(),
        }
    }
}

#[derive(Clone, Default)]
pub struct RuntimeBindings {
    pub ledger: Option<ManagedHandle>,
    pub nodestore: Option<ManagedHandle>,
    pub shamap_store: Option<ManagedHandle>,
    pub resolver: Option<ManagedHandle>,
    pub overlay: Option<ManagedHandle>,
    pub consensus: Option<ManagedHandle>,
    pub server: Option<ManagedHandle>,
    pub validator_site: Option<ManagedHandle>,
    pub perf_log: Option<ManagedHandle>,
    pub grpc: GrpcRuntime,
}

impl std::fmt::Debug for RuntimeBindings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeBindings")
            .field("has_ledger", &self.ledger.is_some())
            .field("has_nodestore", &self.nodestore.is_some())
            .field("has_shamap_store", &self.shamap_store.is_some())
            .field("has_resolver", &self.resolver.is_some())
            .field("has_overlay", &self.overlay.is_some())
            .field("has_consensus", &self.consensus.is_some())
            .field("has_server", &self.server.is_some())
            .field("has_validator_site", &self.validator_site.is_some())
            .field("has_perf_log", &self.perf_log.is_some())
            .field("grpc", &self.grpc)
            .finish()
    }
}

impl RuntimeBindings {
    pub fn fd_required(&self) -> usize {
        self.nodestore
            .as_ref()
            .map_or(0, |component| component.fd_required())
            + self
                .shamap_store
                .as_ref()
                .map_or(0, |component| component.fd_required())
            + self
                .resolver
                .as_ref()
                .map_or(0, |component| component.fd_required())
            + self
                .overlay
                .as_ref()
                .map_or(0, |component| component.fd_required())
            + self
                .ledger
                .as_ref()
                .map_or(0, |component| component.fd_required())
            + self
                .consensus
                .as_ref()
                .map_or(0, |component| component.fd_required())
            + self
                .server
                .as_ref()
                .map_or(0, |component| component.fd_required())
            + self
                .validator_site
                .as_ref()
                .map_or(0, |component| component.fd_required())
            + self
                .perf_log
                .as_ref()
                .map_or(0, |component| component.fd_required())
            + match &self.grpc {
                GrpcRuntime::DisabledExplicit { .. } => 0,
                GrpcRuntime::Enabled(component) => component.fd_required(),
            }
    }
}

pub trait DescriptorLimitProvider {
    fn current_descriptor_limit(&self) -> Option<u64>;
    fn set_descriptor_limit(&self, requested: u64) -> Option<u64>;
}

pub fn adjust_descriptor_limit(required: u64, provider: &dyn DescriptorLimitProvider) -> bool {
    let mut available = provider.current_descriptor_limit().unwrap_or(required);
    if available < required {
        if let Some(updated) = provider.set_descriptor_limit(required) {
            available = updated;
        }
    }
    required <= available
}

pub struct MainRuntime {
    root: ApplicationRoot,
    stop_signal: Arc<(Mutex<bool>, Condvar)>,
    started: AtomicBool,
    shutdown_started: AtomicBool,
}

impl std::fmt::Debug for MainRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MainRuntime")
            .field("root", &self.root)
            .field("started", &self.started.load(Ordering::Acquire))
            .field(
                "shutdown_started",
                &self.shutdown_started.load(Ordering::Acquire),
            )
            .finish()
    }
}

impl MainRuntime {
    pub fn new(root: ApplicationRoot) -> Self {
        Self {
            root,
            stop_signal: Arc::new((Mutex::new(false), Condvar::new())),
            started: AtomicBool::new(false),
            shutdown_started: AtomicBool::new(false),
        }
    }

    pub fn root(&self) -> &ApplicationRoot {
        &self.root
    }

    pub fn transaction_master(&self) -> Arc<TransactionMaster> {
        self.root.transaction_master()
    }

    pub fn canonicalize_transaction(&self, txn: &mut SharedTransaction) {
        self.root.canonicalize_transaction(txn);
    }

    pub fn transaction_json(
        &self,
        transaction: &Transaction,
        options: JsonOptions,
        binary: bool,
    ) -> JsonValue {
        self.root.transaction_json(transaction, options, binary)
    }

    pub fn start(&self) -> Result<(), String> {
        if self.shutdown_started.load(Ordering::Acquire) {
            return Err("runtime has already been shut down".to_owned());
        }
        if self
            .started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("runtime already started".to_owned());
        }

        let mut started_components = Vec::new();
        self.root.load_manager().start();

        if let Some(component) = self.root.runtime_bindings().nodestore.as_ref() {
            if let Err(err) = Self::start_component(component, &mut started_components) {
                self.rollback_start(started_components);
                self.started.store(false, Ordering::Release);
                return Err(err);
            }
        }
        if let Some(component) = self.root.runtime_bindings().shamap_store.as_ref() {
            if let Err(err) = Self::start_component(component, &mut started_components) {
                self.rollback_start(started_components);
                self.started.store(false, Ordering::Release);
                return Err(err);
            }
        }
        if let Some(component) = self.root.runtime_bindings().resolver.as_ref() {
            if let Err(err) = Self::start_component(component, &mut started_components) {
                self.rollback_start(started_components);
                self.started.store(false, Ordering::Release);
                return Err(err);
            }
        }
        if let Some(component) = self.root.runtime_bindings().overlay.as_ref() {
            if let Err(err) = Self::start_component(component, &mut started_components) {
                self.rollback_start(started_components);
                self.started.store(false, Ordering::Release);
                return Err(err);
            }
        }
        if let GrpcRuntime::Enabled(component) = &self.root.runtime_bindings().grpc {
            if let Err(err) = Self::start_component(component, &mut started_components) {
                self.rollback_start(started_components);
                self.started.store(false, Ordering::Release);
                return Err(err);
            }
        }
        if let Some(component) = self.root.runtime_bindings().ledger.as_ref() {
            if let Err(err) = Self::start_component(component, &mut started_components) {
                self.rollback_start(started_components);
                self.started.store(false, Ordering::Release);
                return Err(err);
            }
        }
        if let Some(component) = self.root.runtime_bindings().perf_log.as_ref() {
            if let Err(err) = Self::start_component(component, &mut started_components) {
                self.rollback_start(started_components);
                self.started.store(false, Ordering::Release);
                return Err(err);
            }
        }
        if let Some(component) = self.root.runtime_bindings().validator_site.as_ref() {
            if let Err(err) = Self::start_component(component, &mut started_components) {
                self.rollback_start(started_components);
                self.started.store(false, Ordering::Release);
                return Err(err);
            }
        }
        if let Some(component) = self.root.runtime_bindings().consensus.as_ref() {
            if let Err(err) = Self::start_component(component, &mut started_components) {
                self.rollback_start(started_components);
                self.started.store(false, Ordering::Release);
                return Err(err);
            }
        }
        if let Some(component) = self.root.runtime_bindings().server.as_ref() {
            if let Err(err) = Self::start_component(component, &mut started_components) {
                self.rollback_start(started_components);
                self.started.store(false, Ordering::Release);
                return Err(err);
            }
        }

        Ok(())
    }

    pub fn signal_stop(&self, reason: impl Into<String>) -> bool {
        let first = self.root.signal_stop(reason);
        if first {
            let (lock, condvar) = &*self.stop_signal;
            *lock.lock().expect("stop signal mutex must not be poisoned") = true;
            condvar.notify_all();
        }
        first
    }

    pub fn run(&self) {
        let (lock, condvar) = &*self.stop_signal;
        let mut stop = lock.lock().expect("stop signal mutex must not be poisoned");
        while !*stop {
            stop = condvar
                .wait(stop)
                .expect("stop signal condvar must not be poisoned");
        }
        drop(stop);
        self.shutdown();
    }

    pub fn shutdown(&self) {
        if self.shutdown_started.swap(true, Ordering::AcqRel) {
            return;
        }

        if let Some(component) = self.root.runtime_bindings().validator_site.as_ref() {
            component.stop();
        }
        self.root.load_manager().stop();
        if let Some(component) = self.root.runtime_bindings().shamap_store.as_ref() {
            component.stop();
        }
        self.started.store(false, Ordering::Release);
        self.root.job_queue().stop();

        if let Some(component) = self.root.runtime_bindings().overlay.as_ref() {
            component.stop();
        }
        if let Some(component) = self.root.runtime_bindings().resolver.as_ref() {
            component.stop();
        }
        if let GrpcRuntime::Enabled(component) = &self.root.runtime_bindings().grpc {
            component.stop();
        }
        if let Some(component) = self.root.runtime_bindings().consensus.as_ref() {
            component.stop();
        }
        if let Some(component) = self.root.runtime_bindings().server.as_ref() {
            component.stop();
        }
        if let Some(component) = self.root.runtime_bindings().ledger.as_ref() {
            component.stop();
        }
        if let Some(component) = self.root.runtime_bindings().nodestore.as_ref() {
            component.stop();
        }
        if let Some(component) = self.root.runtime_bindings().perf_log.as_ref() {
            component.stop();
        }
    }

    fn start_component(
        component: &ManagedHandle,
        started_components: &mut Vec<ManagedHandle>,
    ) -> Result<(), String> {
        component.start()?;
        started_components.push(Arc::clone(component));
        Ok(())
    }

    fn rollback_start(&self, started_components: Vec<ManagedHandle>) {
        for component in started_components.into_iter().rev() {
            component.stop();
        }
        self.root.load_manager().stop();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DescriptorLimitProvider, GrpcRuntime, MainRuntime, ManagedComponent, RuntimeBindings,
        adjust_descriptor_limit,
    };
    use crate::state::application_root::ApplicationRoot;
    use crate::{
        SHAMapStore, SHAMapStoreCloseTimeProvider, SHAMapStoreComponent,
        SHAMapStoreComponentRuntime, SHAMapStoreHealthRuntime, SHAMapStoreOperatingMode,
        SHAMapStoreRuntime, SHAMapStoreService, SharedSHAMapStoreHealthState,
    };
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct RecordingComponent {
        name: &'static str,
        fail_on_start: bool,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingComponent {
        fn new(name: &'static str, events: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                name,
                fail_on_start: false,
                events,
            }
        }

        fn failing(name: &'static str, events: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                name,
                fail_on_start: true,
                events,
            }
        }
    }

    impl ManagedComponent for RecordingComponent {
        fn start(&self) -> Result<(), String> {
            self.events
                .lock()
                .expect("events mutex")
                .push(format!("{}:start", self.name));
            if self.fail_on_start {
                return Err(format!("{} failed to start", self.name));
            }
            Ok(())
        }

        fn stop(&self) {
            self.events
                .lock()
                .expect("events mutex")
                .push(format!("{}:stop", self.name));
        }

        fn fd_required(&self) -> usize {
            4
        }
    }

    #[derive(Debug)]
    struct FixedCloseTimeProvider {
        now_close_time: AtomicU32,
    }

    impl FixedCloseTimeProvider {
        fn new(now_close_time: u32) -> Self {
            Self {
                now_close_time: AtomicU32::new(now_close_time),
            }
        }
    }

    impl SHAMapStoreCloseTimeProvider for FixedCloseTimeProvider {
        fn current_close_time(&self) -> u32 {
            self.now_close_time.load(Ordering::Acquire)
        }
    }

    struct ServiceRuntime {
        stopping: Arc<AtomicBool>,
    }

    impl SHAMapStoreRuntime for ServiceRuntime {
        fn start_background_work(&mut self) {}

        fn stop_background_work(&mut self) {}

        fn minimum_sql_seq(&self) -> Option<u32> {
            None
        }
    }

    impl SHAMapStoreHealthRuntime for ServiceRuntime {
        fn is_stopping(&self) -> bool {
            self.stopping.load(Ordering::Acquire)
        }

        fn operating_mode(&self) -> SHAMapStoreOperatingMode {
            SHAMapStoreOperatingMode::Full
        }

        fn validated_ledger_age(&self) -> Duration {
            Duration::from_secs(1)
        }
    }

    impl SHAMapStoreComponentRuntime for ServiceRuntime {}

    struct StubLimitProvider {
        current: Option<u64>,
        updated: Option<u64>,
    }

    impl DescriptorLimitProvider for StubLimitProvider {
        fn current_descriptor_limit(&self) -> Option<u64> {
            self.current
        }

        fn set_descriptor_limit(&self, requested: u64) -> Option<u64> {
            self.updated.or(Some(requested))
        }
    }

    #[test]
    fn runtime_shutdown_uses_explicit_grpc_disabled_path() {
        let mut root = ApplicationRoot::new(0).expect("root");
        let events = Arc::new(Mutex::new(Vec::new()));
        let server = Arc::new(RecordingComponent::new("server", Arc::clone(&events)));
        root.set_runtime_bindings(RuntimeBindings {
            server: Some(server.clone()),
            grpc: GrpcRuntime::DisabledExplicit {
                reason: "disabled in parity test".to_owned(),
            },
            ..RuntimeBindings::default()
        });

        let runtime = MainRuntime::new(root);
        runtime.start().expect("runtime should start");
        runtime.signal_stop("done");
        runtime.shutdown();

        assert_eq!(
            server.events.lock().expect("events mutex").as_slice(),
            &["server:start".to_owned(), "server:stop".to_owned()]
        );
    }

    #[test]
    fn runtime_start_and_shutdown_follow_the_bound_component_order() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut root = ApplicationRoot::new(0).expect("root");
        root.set_runtime_bindings(RuntimeBindings {
            shamap_store: Some(Arc::new(RecordingComponent::new(
                "shamap_store",
                Arc::clone(&events),
            ))),
            nodestore: Some(Arc::new(RecordingComponent::new(
                "nodestore",
                Arc::clone(&events),
            ))),
            ledger: Some(Arc::new(RecordingComponent::new(
                "ledger",
                Arc::clone(&events),
            ))),
            consensus: Some(Arc::new(RecordingComponent::new(
                "consensus",
                Arc::clone(&events),
            ))),
            server: Some(Arc::new(RecordingComponent::new(
                "server",
                Arc::clone(&events),
            ))),
            resolver: None,
            overlay: Some(Arc::new(RecordingComponent::new(
                "overlay",
                Arc::clone(&events),
            ))),
            validator_site: None,
            perf_log: None,
            grpc: GrpcRuntime::Enabled(Arc::new(RecordingComponent::new(
                "grpc",
                Arc::clone(&events),
            ))),
        });

        let runtime = MainRuntime::new(root);
        runtime.start().expect("runtime should start");
        runtime.shutdown();

        assert_eq!(
            events.lock().expect("events mutex").as_slice(),
            &[
                "nodestore:start".to_owned(),
                "shamap_store:start".to_owned(),
                "overlay:start".to_owned(),
                "grpc:start".to_owned(),
                "ledger:start".to_owned(),
                "consensus:start".to_owned(),
                "server:start".to_owned(),
                "shamap_store:stop".to_owned(),
                "overlay:stop".to_owned(),
                "grpc:stop".to_owned(),
                "consensus:stop".to_owned(),
                "server:stop".to_owned(),
                "ledger:stop".to_owned(),
                "nodestore:stop".to_owned(),
            ]
        );
    }

    #[test]
    fn runtime_start_rolls_back_started_components_when_a_later_start_fails() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut root = ApplicationRoot::new(0).expect("root");
        root.set_runtime_bindings(RuntimeBindings {
            nodestore: Some(Arc::new(RecordingComponent::new(
                "nodestore",
                Arc::clone(&events),
            ))),
            ledger: Some(Arc::new(RecordingComponent::new(
                "ledger",
                Arc::clone(&events),
            ))),
            shamap_store: Some(Arc::new(RecordingComponent::failing(
                "shamap_store",
                Arc::clone(&events),
            ))),
            consensus: Some(Arc::new(RecordingComponent::failing(
                "consensus",
                Arc::clone(&events),
            ))),
            server: Some(Arc::new(RecordingComponent::new(
                "server",
                Arc::clone(&events),
            ))),
            validator_site: None,
            perf_log: None,
            ..RuntimeBindings::default()
        });

        let runtime = MainRuntime::new(root);
        let err = runtime.start().expect_err("shamap store start should fail");

        assert_eq!(err, "shamap_store failed to start");
        assert_eq!(
            events.lock().expect("events mutex").as_slice(),
            &[
                "nodestore:start".to_owned(),
                "shamap_store:start".to_owned(),
                "nodestore:stop".to_owned(),
            ]
        );
    }

    #[test]
    fn descriptor_adjustment_shape() {
        assert!(adjust_descriptor_limit(
            64,
            &StubLimitProvider {
                current: Some(32),
                updated: Some(64),
            }
        ));
        assert!(!adjust_descriptor_limit(
            64,
            &StubLimitProvider {
                current: Some(32),
                updated: Some(48),
            }
        ));
    }

    #[test]
    fn runtime_signal_stop_leaves_shamap_store_service_running_until_shutdown() {
        let health = Arc::new(SharedSHAMapStoreHealthState::new(Arc::new(
            FixedCloseTimeProvider::new(120),
        )));
        let component = Arc::new(SHAMapStoreComponent::new(
            SHAMapStore::new(256, false, 9),
            Box::new(ServiceRuntime {
                stopping: Arc::new(AtomicBool::new(false)),
            }),
            None,
        ));
        let service = Arc::new(SHAMapStoreService::new(component, health.clone()));
        let mut root = ApplicationRoot::new(0).expect("root");
        root.attach_shamap_store_service(service);

        let runtime = MainRuntime::new(root);
        assert!(runtime.signal_stop("shutdown"));
        assert!(!health.is_stopping());
        runtime.shutdown();
        assert!(health.is_stopping());
    }
}
