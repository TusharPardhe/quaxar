use crate::shamap::shamap_store::SHAMapStoreRuntime;
use crate::shamap::shamap_store_backend::make_shamap_store_rotating_backend;
use crate::shamap::shamap_store_component::SHAMapStoreComponentRuntime;
use crate::shamap::shamap_store_copy::SHAMapStoreCopyDisposition;
use crate::shamap::shamap_store_health::{
    SHAMapStoreHealthRuntime, SHAMapStoreOperatingMode, SharedSHAMapStoreHealthState,
};
use crate::shamap::shamap_store_relational::SHAMapStoreRelationalRuntime;
use crate::{NodeFamily, TransactionMaster};
use basics::base_uint::Uint256;
use basics::tagged_cache::CacheClock;
use ledger::Ledger;
use ledger::LedgerMaster;
use nodestore::{Backend, DatabaseRotating, FetchType, Manager, NodeStoreJournal, Scheduler};
use shamap::family::{FullBelowCache, MissingNodeReporter, SHAMapNodeFetcher};
use shamap::traversal::TraversalError;
use std::hash::BuildHasher;
use std::sync::Arc;
use std::time::Duration;

pub trait SHAMapStoreLedgerRuntime: Send + Sync {
    fn clear_prior_ledgers(&self, last_rotated: u32);
    fn clear_online_delete_caches(&self, validated_seq: u32);
}

pub trait SHAMapStoreNodeFamilyCacheRuntime: Send + Sync {
    fn tree_node_cache_keys(&self) -> Vec<Uint256>;
    fn clear_full_below_cache(&self);
    fn visit_state_map_hashes(
        &self,
        ledger: &Ledger,
        visit: &mut dyn FnMut(Uint256) -> bool,
    ) -> Result<(), TraversalError>;
}

pub trait SHAMapStoreTransactionCacheRuntime: Send + Sync {
    fn cache_keys(&self) -> Vec<Uint256>;
}

pub trait SHAMapStoreNodeStoreRuntime: Send + Sync {
    fn fetch_node_object(&self, hash: &Uint256, ledger_seq: u32) -> bool;
    fn rotate_with(&self, new_backend: Box<dyn Backend>) -> (String, String);
}

pub trait SHAMapStoreRotatingBackendFactory: Send + Sync {
    fn make_backend(&self) -> Result<Box<dyn Backend>, String>;
}

pub trait SHAMapStoreCopyRuntime: Send + Sync {
    fn copy_validated_ledger(
        &self,
        _validated_ledger: Arc<Ledger>,
        _node_family: &dyn SHAMapStoreNodeFamilyCacheRuntime,
        _node_store: &dyn SHAMapStoreNodeStoreRuntime,
        _runtime: &mut dyn SHAMapStoreComponentRuntime,
        _health_policy: crate::SHAMapStoreHealthPolicy,
    ) -> Result<SHAMapStoreCopyDisposition, String> {
        Ok(SHAMapStoreCopyDisposition::Completed { node_count: 0 })
    }
}

#[derive(Debug, Default)]
pub struct NullSHAMapStoreCopyRuntime;

impl SHAMapStoreCopyRuntime for NullSHAMapStoreCopyRuntime {}

#[derive(Clone)]
pub struct ConfiguredSHAMapStoreBackendFactory {
    manager: Arc<dyn Manager>,
    node_db: basics::basic_config::Section,
    burst_size: usize,
    scheduler: Arc<dyn Scheduler>,
    journal: Arc<dyn NodeStoreJournal>,
}

impl ConfiguredSHAMapStoreBackendFactory {
    pub fn new(
        manager: Arc<dyn Manager>,
        node_db: basics::basic_config::Section,
        burst_size: usize,
        scheduler: Arc<dyn Scheduler>,
        journal: Arc<dyn NodeStoreJournal>,
    ) -> Self {
        Self {
            manager,
            node_db,
            burst_size,
            scheduler,
            journal,
        }
    }
}

impl std::fmt::Debug for ConfiguredSHAMapStoreBackendFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfiguredSHAMapStoreBackendFactory")
            .field("burst_size", &self.burst_size)
            .field("node_db", &self.node_db)
            .finish()
    }
}

impl SHAMapStoreRotatingBackendFactory for ConfiguredSHAMapStoreBackendFactory {
    fn make_backend(&self) -> Result<Box<dyn Backend>, String> {
        make_shamap_store_rotating_backend(
            self.manager.as_ref(),
            &self.node_db,
            self.burst_size,
            Arc::clone(&self.scheduler),
            Arc::clone(&self.journal),
            None,
        )
    }
}

pub struct SHAMapStoreAppRuntime {
    ledger: Arc<dyn SHAMapStoreLedgerRuntime>,
    node_family: Arc<dyn SHAMapStoreNodeFamilyCacheRuntime>,
    transaction_master: Arc<dyn SHAMapStoreTransactionCacheRuntime>,
    node_store: Arc<dyn SHAMapStoreNodeStoreRuntime>,
    backend_factory: Arc<dyn SHAMapStoreRotatingBackendFactory>,
    relational: Option<Arc<dyn SHAMapStoreRelationalRuntime>>,
    copy_runtime: Arc<dyn SHAMapStoreCopyRuntime>,
    stopping: bool,
    operating_mode: SHAMapStoreOperatingMode,
    validated_ledger_age: Duration,
    background_starts: usize,
    background_stops: usize,
    prepared_backend: Option<Box<dyn Backend>>,
    health_state: Option<Arc<SharedSHAMapStoreHealthState>>,
}

impl std::fmt::Debug for SHAMapStoreAppRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SHAMapStoreAppRuntime")
            .field("has_relational", &self.relational.is_some())
            .field("stopping", &self.stopping)
            .field("operating_mode", &self.operating_mode)
            .field("validated_ledger_age", &self.validated_ledger_age)
            .field("background_starts", &self.background_starts)
            .field("background_stops", &self.background_stops)
            .field("prepared_backend", &self.prepared_backend.is_some())
            .finish()
    }
}

impl SHAMapStoreAppRuntime {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ledger: Arc<dyn SHAMapStoreLedgerRuntime>,
        node_family: Arc<dyn SHAMapStoreNodeFamilyCacheRuntime>,
        transaction_master: Arc<dyn SHAMapStoreTransactionCacheRuntime>,
        node_store: Arc<dyn SHAMapStoreNodeStoreRuntime>,
        backend_factory: Arc<dyn SHAMapStoreRotatingBackendFactory>,
        relational: Option<Arc<dyn SHAMapStoreRelationalRuntime>>,
        copy_runtime: Arc<dyn SHAMapStoreCopyRuntime>,
    ) -> Self {
        Self::with_health_state(
            ledger,
            node_family,
            transaction_master,
            node_store,
            backend_factory,
            relational,
            copy_runtime,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_health_state(
        ledger: Arc<dyn SHAMapStoreLedgerRuntime>,
        node_family: Arc<dyn SHAMapStoreNodeFamilyCacheRuntime>,
        transaction_master: Arc<dyn SHAMapStoreTransactionCacheRuntime>,
        node_store: Arc<dyn SHAMapStoreNodeStoreRuntime>,
        backend_factory: Arc<dyn SHAMapStoreRotatingBackendFactory>,
        relational: Option<Arc<dyn SHAMapStoreRelationalRuntime>>,
        copy_runtime: Arc<dyn SHAMapStoreCopyRuntime>,
        health_state: Arc<SharedSHAMapStoreHealthState>,
    ) -> Self {
        Self::with_health_state(
            ledger,
            node_family,
            transaction_master,
            node_store,
            backend_factory,
            relational,
            copy_runtime,
            Some(health_state),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn with_health_state(
        ledger: Arc<dyn SHAMapStoreLedgerRuntime>,
        node_family: Arc<dyn SHAMapStoreNodeFamilyCacheRuntime>,
        transaction_master: Arc<dyn SHAMapStoreTransactionCacheRuntime>,
        node_store: Arc<dyn SHAMapStoreNodeStoreRuntime>,
        backend_factory: Arc<dyn SHAMapStoreRotatingBackendFactory>,
        relational: Option<Arc<dyn SHAMapStoreRelationalRuntime>>,
        copy_runtime: Arc<dyn SHAMapStoreCopyRuntime>,
        health_state: Option<Arc<SharedSHAMapStoreHealthState>>,
    ) -> Self {
        Self {
            ledger,
            node_family,
            transaction_master,
            node_store,
            backend_factory,
            relational,
            copy_runtime,
            stopping: false,
            operating_mode: SHAMapStoreOperatingMode::Other,
            validated_ledger_age: Duration::default(),
            background_starts: 0,
            background_stops: 0,
            prepared_backend: None,
            health_state,
        }
    }

    pub fn set_stopping(&mut self, stopping: bool) {
        self.stopping = stopping;
        if let Some(health_state) = &self.health_state {
            health_state.set_stopping(stopping);
        }
    }

    pub fn set_operating_mode(&mut self, operating_mode: SHAMapStoreOperatingMode) {
        self.operating_mode = operating_mode;
        if let Some(health_state) = &self.health_state {
            health_state.set_operating_mode(operating_mode);
        }
    }

    pub fn set_validated_ledger_age(&mut self, validated_ledger_age: Duration) {
        self.validated_ledger_age = validated_ledger_age;
        if let Some(health_state) = &self.health_state {
            health_state.set_validated_ledger_age(validated_ledger_age);
        }
    }

    pub fn background_counts(&self) -> (usize, usize) {
        (self.background_starts, self.background_stops)
    }

    fn freshen_keys(&self, keys: impl IntoIterator<Item = Uint256>) {
        for hash in keys {
            let _ = self.node_store.fetch_node_object(&hash, 0);
            if self.stopping {
                return;
            }
        }
    }
}

impl SHAMapStoreRuntime for SHAMapStoreAppRuntime {
    fn start_background_work(&mut self) {
        self.stopping = false;
        self.background_starts += 1;
    }

    fn stop_background_work(&mut self) {
        self.stopping = true;
        self.background_stops += 1;
    }

    fn minimum_sql_seq(&self) -> Option<u32> {
        self.relational
            .as_ref()
            .and_then(|relational| relational.minimum_sql_seq())
    }
}

impl SHAMapStoreHealthRuntime for SHAMapStoreAppRuntime {
    fn is_stopping(&self) -> bool {
        self.health_state
            .as_ref()
            .map_or(self.stopping, |health_state| health_state.is_stopping())
    }

    fn operating_mode(&self) -> SHAMapStoreOperatingMode {
        self.health_state
            .as_ref()
            .map_or(self.operating_mode, |health_state| {
                health_state.operating_mode()
            })
    }

    fn validated_ledger_age(&self) -> Duration {
        self.health_state
            .as_ref()
            .map_or(self.validated_ledger_age, |health_state| {
                health_state.validated_ledger_age()
            })
    }
}

impl SHAMapStoreComponentRuntime for SHAMapStoreAppRuntime {
    fn clear_prior(&mut self, last_rotated: u32) -> Result<(), String> {
        self.ledger.clear_prior_ledgers(last_rotated);
        if self.stopping {
            return Ok(());
        }
        if let Some(relational) = &self.relational {
            relational.clear_prior(last_rotated, &|| self.stopping)?;
        }
        Ok(())
    }

    fn copy_validated_ledger(
        &mut self,
        validated_ledger: Arc<Ledger>,
        health_policy: crate::SHAMapStoreHealthPolicy,
    ) -> Result<SHAMapStoreCopyDisposition, String> {
        let copy_runtime = Arc::clone(&self.copy_runtime);
        let node_family = Arc::clone(&self.node_family);
        let node_store = Arc::clone(&self.node_store);
        copy_runtime.copy_validated_ledger(
            validated_ledger,
            node_family.as_ref(),
            node_store.as_ref(),
            self,
            health_policy,
        )
    }

    fn freshen_caches(&mut self) -> Result<(), String> {
        self.freshen_keys(self.node_family.tree_node_cache_keys());
        if self.stopping {
            return Ok(());
        }
        self.freshen_keys(self.transaction_master.cache_keys());
        Ok(())
    }

    fn prepare_rotation(&mut self) -> Result<(), String> {
        self.prepared_backend = Some(self.backend_factory.make_backend()?);
        Ok(())
    }

    fn rotate_backends(&mut self) -> Result<(String, String), String> {
        let Some(new_backend) = self.prepared_backend.take() else {
            return Err("rotation backend was not prepared".to_owned());
        };
        Ok(self.node_store.rotate_with(new_backend))
    }

    fn clear_caches(&mut self, validated_seq: u32) -> Result<(), String> {
        self.ledger.clear_online_delete_caches(validated_seq);
        self.node_family.clear_full_below_cache();
        Ok(())
    }
}

impl<C, S> SHAMapStoreLedgerRuntime for LedgerMaster<C, S>
where
    C: CacheClock + Clone + Send + Sync + 'static,
    S: BuildHasher + Clone + Send + Sync + 'static,
{
    fn clear_prior_ledgers(&self, last_rotated: u32) {
        LedgerMaster::clear_prior_ledgers(self, last_rotated);
    }

    fn clear_online_delete_caches(&self, validated_seq: u32) {
        LedgerMaster::clear_cached_ledger_entries_prior(self, validated_seq);
    }
}

impl<C, S, FB, F, MR, NS> SHAMapStoreNodeFamilyCacheRuntime for NodeFamily<C, S, FB, F, MR, NS>
where
    C: CacheClock + Send + Sync + 'static,
    S: BuildHasher + Clone + Send + Sync + 'static,
    FB: FullBelowCache + Send + Sync + 'static,
    F: SHAMapNodeFetcher + Send + Sync + 'static,
    MR: MissingNodeReporter + Send + Sync + 'static,
    NS: Send + Sync + 'static,
{
    fn tree_node_cache_keys(&self) -> Vec<Uint256> {
        NodeFamily::tree_node_cache_keys(self)
    }

    fn clear_full_below_cache(&self) {
        NodeFamily::clear_full_below_cache(self);
    }

    fn visit_state_map_hashes(
        &self,
        ledger: &Ledger,
        visit: &mut dyn FnMut(Uint256) -> bool,
    ) -> Result<(), TraversalError> {
        let family = self.shared_family();
        ledger
            .state_map()
            .visit_nodes_with_family(family.as_ref(), &mut |node| {
                visit(*node.get_hash().as_uint256())
            })
    }
}

impl<C> SHAMapStoreTransactionCacheRuntime for TransactionMaster<C>
where
    C: CacheClock + Send + Sync + 'static,
{
    fn cache_keys(&self) -> Vec<Uint256> {
        TransactionMaster::cache_keys(self)
    }
}

impl<T> SHAMapStoreNodeStoreRuntime for T
where
    T: DatabaseRotating + ?Sized,
{
    fn fetch_node_object(&self, hash: &Uint256, ledger_seq: u32) -> bool {
        nodestore::Database::fetch_node_object(self, hash, ledger_seq, FetchType::Synchronous, true)
            .is_some()
    }

    fn rotate_with(&self, new_backend: Box<dyn Backend>) -> (String, String) {
        let mut next_names = None;
        nodestore::DatabaseRotating::rotate(
            self,
            new_backend,
            &mut |writable_name, archive_name| {
                next_names = Some((writable_name.to_owned(), archive_name.to_owned()));
            },
        );
        next_names.expect("rotation callback must set next backend names")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NullSHAMapStoreCopyRuntime, SHAMapStoreAppRuntime, SHAMapStoreLedgerRuntime,
        SHAMapStoreNodeFamilyCacheRuntime, SHAMapStoreNodeStoreRuntime,
        SHAMapStoreRotatingBackendFactory, SHAMapStoreTransactionCacheRuntime,
    };
    use crate::{
        SHAMapStoreComponentRuntime, SHAMapStoreHealthRuntime, SHAMapStoreOperatingMode,
        SHAMapStoreRuntime,
    };
    use basics::base_uint::Uint256;
    use ledger::Ledger;
    use nodestore::Backend;
    use shamap::traversal::TraversalError;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Default)]
    struct RecordingLedgerRuntime {
        events: Mutex<Vec<String>>,
    }

    impl SHAMapStoreLedgerRuntime for RecordingLedgerRuntime {
        fn clear_prior_ledgers(&self, last_rotated: u32) {
            self.events
                .lock()
                .expect("events mutex must not be poisoned")
                .push(format!("ledger-clear-prior:{last_rotated}"));
        }

        fn clear_online_delete_caches(&self, validated_seq: u32) {
            self.events
                .lock()
                .expect("events mutex must not be poisoned")
                .push(format!("ledger-clear-caches:{validated_seq}"));
        }
    }

    #[derive(Default)]
    struct RecordingNodeFamilyRuntime {
        events: Mutex<Vec<String>>,
        keys: Vec<Uint256>,
    }

    impl SHAMapStoreNodeFamilyCacheRuntime for RecordingNodeFamilyRuntime {
        fn tree_node_cache_keys(&self) -> Vec<Uint256> {
            self.keys.clone()
        }

        fn clear_full_below_cache(&self) {
            self.events
                .lock()
                .expect("events mutex must not be poisoned")
                .push("clear-full-below".to_owned());
        }

        fn visit_state_map_hashes(
            &self,
            _ledger: &Ledger,
            _visit: &mut dyn FnMut(Uint256) -> bool,
        ) -> Result<(), TraversalError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingTransactionRuntime {
        keys: Vec<Uint256>,
    }

    impl SHAMapStoreTransactionCacheRuntime for RecordingTransactionRuntime {
        fn cache_keys(&self) -> Vec<Uint256> {
            self.keys.clone()
        }
    }

    #[derive(Default)]
    struct RecordingNodeStoreRuntime {
        fetches: Mutex<Vec<Uint256>>,
        rotations: Mutex<Vec<String>>,
    }

    impl SHAMapStoreNodeStoreRuntime for RecordingNodeStoreRuntime {
        fn fetch_node_object(&self, hash: &Uint256, _ledger_seq: u32) -> bool {
            self.fetches
                .lock()
                .expect("fetches mutex must not be poisoned")
                .push(*hash);
            true
        }

        fn rotate_with(&self, new_backend: Box<dyn Backend>) -> (String, String) {
            let name = new_backend.get_name();
            self.rotations
                .lock()
                .expect("rotations mutex must not be poisoned")
                .push(name.clone());
            (name, "archive.current".to_owned())
        }
    }

    struct TestBackend(&'static str);

    impl Backend for TestBackend {
        fn get_name(&self) -> String {
            self.0.to_owned()
        }

        fn open(&self, _create_if_missing: bool) -> Result<(), String> {
            Ok(())
        }

        fn is_open(&self) -> bool {
            true
        }

        fn close(&self) -> Result<(), String> {
            Ok(())
        }

        fn fetch(&self, _key: &Uint256) -> (Option<Arc<nodestore::NodeObject>>, nodestore::Status) {
            (None, nodestore::Status::NotFound)
        }

        fn fetch_batch(
            &self,
            _hashes: &[Uint256],
        ) -> (Vec<Option<Arc<nodestore::NodeObject>>>, nodestore::Status) {
            (Vec::new(), nodestore::Status::NotFound)
        }

        fn store(&self, _object: Arc<nodestore::NodeObject>) {}

        fn store_batch(&self, _batch: &nodestore::Batch) {}

        fn for_each(&self, _callback: &mut dyn FnMut(Arc<nodestore::NodeObject>)) {}

        fn get_write_load(&self) -> i32 {
            0
        }

        fn set_delete_path(&self) {}

        fn verify(&self) {}

        fn fd_required(&self) -> i32 {
            1
        }

        fn sync(&self) {}
    }

    #[derive(Debug)]
    struct RecordingFactory;

    impl SHAMapStoreRotatingBackendFactory for RecordingFactory {
        fn make_backend(&self) -> Result<Box<dyn Backend>, String> {
            Ok(Box::new(TestBackend("writable.next")))
        }
    }

    #[test]
    fn app_runtime_freshens_tree_cache_before_transaction_cache_and_clears_caches_in() {
        let ledger: Arc<RecordingLedgerRuntime> = Arc::default();
        let node_family = Arc::new(RecordingNodeFamilyRuntime {
            events: Mutex::default(),
            keys: vec![
                Uint256::from_array([0x11; 32]),
                Uint256::from_array([0x22; 32]),
            ],
        });
        let transactions = Arc::new(RecordingTransactionRuntime {
            keys: vec![Uint256::from_array([0x33; 32])],
        });
        let node_store: Arc<RecordingNodeStoreRuntime> = Arc::default();

        let mut runtime = SHAMapStoreAppRuntime::new(
            ledger.clone(),
            node_family.clone(),
            transactions,
            node_store.clone(),
            Arc::new(RecordingFactory),
            None,
            Arc::new(NullSHAMapStoreCopyRuntime),
        );

        runtime.freshen_caches().expect("freshen");
        runtime.clear_caches(1156).expect("clear caches");

        assert_eq!(
            *node_store
                .fetches
                .lock()
                .expect("fetches mutex must not be poisoned"),
            vec![
                Uint256::from_array([0x11; 32]),
                Uint256::from_array([0x22; 32]),
                Uint256::from_array([0x33; 32]),
            ]
        );
        assert_eq!(
            *ledger
                .events
                .lock()
                .expect("events mutex must not be poisoned"),
            vec!["ledger-clear-caches:1156".to_owned()]
        );
        assert_eq!(
            *node_family
                .events
                .lock()
                .expect("events mutex must not be poisoned"),
            vec!["clear-full-below".to_owned()]
        );
    }

    #[test]
    fn app_runtime_prepares_and_rotates_backends() {
        let mut runtime = SHAMapStoreAppRuntime::new(
            Arc::new(RecordingLedgerRuntime::default()),
            Arc::new(RecordingNodeFamilyRuntime::default()),
            Arc::new(RecordingTransactionRuntime::default()),
            Arc::new(RecordingNodeStoreRuntime::default()),
            Arc::new(RecordingFactory),
            None,
            Arc::new(NullSHAMapStoreCopyRuntime),
        );

        runtime.prepare_rotation().expect("prepare");
        let result = runtime.rotate_backends().expect("rotate");
        assert_eq!(
            result,
            ("writable.next".to_owned(), "archive.current".to_owned())
        );
    }

    #[test]
    fn app_runtime_tracks_health_and_background_lifecycle_state() {
        let mut runtime = SHAMapStoreAppRuntime::new(
            Arc::new(RecordingLedgerRuntime::default()),
            Arc::new(RecordingNodeFamilyRuntime::default()),
            Arc::new(RecordingTransactionRuntime::default()),
            Arc::new(RecordingNodeStoreRuntime::default()),
            Arc::new(RecordingFactory),
            None,
            Arc::new(NullSHAMapStoreCopyRuntime),
        );

        runtime.set_operating_mode(SHAMapStoreOperatingMode::Full);
        runtime.set_validated_ledger_age(Duration::from_secs(5));
        runtime.start_background_work();
        runtime.stop_background_work();

        assert!(runtime.is_stopping());
        assert_eq!(runtime.operating_mode(), SHAMapStoreOperatingMode::Full);
        assert_eq!(runtime.validated_ledger_age(), Duration::from_secs(5));
        assert_eq!(runtime.background_counts(), (1, 1));
    }
}
