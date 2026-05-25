use std::cell::Cell;
use std::ptr;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use xrpl_core::{
    ClosureCounter, ConfigSection, FixedNetworkIdService, Job, JobType, JobTypeData, JobTypeInfo,
    JobTypes, NetworkIDService, PeerReservation, PeerReservationTable, SECTION_NODE_SIZE,
    SECTION_PORT_GRPC, SECTION_SERVER_DOMAIN, ServiceRegistry, StartUpType,
};

#[derive(Debug, PartialEq, Eq)]
struct TrackedString {
    copies: usize,
    moves: usize,
    value: String,
}

impl TrackedString {
    fn new(value: &str) -> Self {
        Self {
            copies: 0,
            moves: 0,
            value: value.to_owned(),
        }
    }

    fn push_suffix(&mut self, suffix: &str) {
        self.value.push_str(suffix);
    }

    fn with_suffix(&self, suffix: &str) -> Self {
        let mut next = self.clone();
        next.value.push_str(suffix);
        next
    }
}

impl Clone for TrackedString {
    fn clone(&self) -> Self {
        Self {
            copies: self.copies + 1,
            moves: self.moves,
            value: self.value.clone(),
        }
    }
}

#[test]
fn job_types_match_cpp_catalog_entries_and_special_rules() {
    let types = JobTypes::instance();
    assert_eq!(types.size(), 46);
    assert_eq!(JobTypes::name(JobType::Pack), "makeFetchPack");
    assert_eq!(types.get(JobType::Rpc).name(), "RPC");
    assert_eq!(types.get(JobType::Peer).limit(), 0);
    assert!(types.get(JobType::Peer).special());
    assert_eq!(types.get(JobType::Batch).limit(), i32::MAX);
    assert_eq!(types.get(JobType::NsWrite).name(), "WriteNode");
    assert_eq!(types.get(JobType::Invalid).name(), "invalid");
}

#[test]
fn job_type_info_and_data_preserve_cpp_static_metadata() {
    let info = JobTypeInfo::new(
        JobType::Transaction,
        "transaction",
        i32::MAX,
        Duration::from_millis(250),
        Duration::from_millis(1_000),
    );
    assert_eq!(info.type_(), JobType::Transaction);
    assert_eq!(info.name(), "transaction");
    assert!(!info.special());
    assert_eq!(info.get_average_latency(), Duration::from_millis(250));
    assert_eq!(info.get_peak_latency(), Duration::from_millis(1_000));

    let data = JobTypeData::new(info);
    assert_eq!(data.name(), "transaction");
    assert_eq!(data.type_(), JobType::Transaction);
    assert_eq!(data.waiting, 0);
    assert_eq!(data.running, 0);
    assert_eq!(data.deferred, 0);
    assert_eq!(data.stats().average_latency, Duration::from_millis(250));
    assert_eq!(data.stats().peak_latency, Duration::from_millis(1_000));
}

#[test]
fn jobs_keep_cpp_priority_ordering_and_one_shot_execution() {
    let lower_type = Job::new(JobType::Pack, 1);
    let higher_type = Job::new(JobType::Accept, 1);
    assert!(higher_type < lower_type);
    assert!(lower_type > higher_type);

    let earlier_index = Job::new(JobType::Client, 1);
    let later_index = Job::new(JobType::Client, 2);
    assert!(earlier_index < later_index);
    assert!(later_index > earlier_index);

    let calls = Arc::new(AtomicUsize::new(0));
    let mut job = Job::new_with_closure(JobType::Client, "run-once", 7, {
        let calls = Arc::clone(&calls);
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
        }
    });

    assert_eq!(job.get_type(), JobType::Client);
    assert_eq!(job.name(), "run-once");
    assert!(job.queue_time().is_some());
    job.do_job();
    job.do_job();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn closure_counter_join_and_wrap_contract() {
    let counter = ClosureCounter::<fn() -> ()>::new();
    assert_eq!(counter.count(), 0);
    assert!(!counter.joined());

    let evidence = Arc::new(AtomicUsize::new(0));
    let mut wrapped = counter
        .wrap({
            let evidence = Arc::clone(&evidence);
            move || {
                evidence.fetch_add(1, Ordering::SeqCst);
            }
        })
        .expect("wrap before join");
    assert_eq!(counter.count(), 1);

    wrapped();
    wrapped();
    assert_eq!(evidence.load(Ordering::SeqCst), 2);

    let timeout_called = Arc::new(AtomicUsize::new(0));
    let timeout_called_clone = Arc::clone(&timeout_called);
    let clone = wrapped.clone();
    drop(wrapped);
    assert_eq!(counter.count(), 1);

    let join_thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(10));
        drop(clone);
    });
    counter.join(
        "closure-counter-test",
        Duration::from_millis(1),
        move || {
            timeout_called_clone.fetch_add(1, Ordering::SeqCst);
        },
    );
    join_thread.join().expect("join thread");

    assert_eq!(counter.count(), 0);
    assert!(counter.joined());
    assert_eq!(timeout_called.load(Ordering::SeqCst), 1);
    assert!(counter.wrap(|| {}).is_none());
}

#[test]
fn closure_counter_supports_zero_one_and_two_argument_closures() {
    let zero = ClosureCounter::<fn() -> i32>::new();
    assert_eq!(zero.wrap(|| 7).expect("zero-arg wrap")(), 7);

    let one = ClosureCounter::<fn(i32) -> i32>::new();
    assert_eq!(one.wrap(|x| x + 1).expect("one-arg wrap")(5), 6);

    let two = ClosureCounter::<fn(i32, i32) -> i32>::new();
    assert_eq!(two.wrap(|a, b| a + b).expect("two-arg wrap")(2, 8), 10);
}

#[test]
fn closure_counter_mutable_closure_and_argument_forwarding_shape() {
    let set_counter = ClosureCounter::<fn(i32) -> ()>::new();
    let evidence = Cell::new(0);
    {
        let mut wrapped_set = set_counter
            .wrap(|value| {
                evidence.set(value);
            })
            .expect("mutable one-arg wrap");
        wrapped_set(5);
        assert_eq!(evidence.get(), 5);
        wrapped_set(11);
        assert_eq!(evidence.get(), 11);
    }

    let by_value = ClosureCounter::<fn(TrackedString) -> TrackedString>::new();
    let source = TrackedString::new("value");
    let result = {
        let mut wrapped_value = by_value
            .wrap(|mut input| {
                input.push_suffix("!");
                input
            })
            .expect("by-value wrap");
        wrapped_value(source.clone())
    };
    assert_eq!(result.copies, 1);
    assert_eq!(result.moves, 0);
    assert_eq!(result.value, "value!");
    assert_eq!(source.value, "value");

    let by_const_ref = ClosureCounter::<fn(&TrackedString) -> TrackedString>::new();
    let source = TrackedString::new("const lvalue");
    let result = {
        let mut wrapped_const_ref = by_const_ref
            .wrap(|input| input.with_suffix("!"))
            .expect("const-ref wrap");
        wrapped_const_ref(&source)
    };
    assert_eq!(result.copies, 1);
    assert_eq!(result.moves, 0);
    assert_eq!(result.value, "const lvalue!");
    assert_eq!(source.value, "const lvalue");

    let by_mut_ref = ClosureCounter::<fn(&mut TrackedString) -> TrackedString>::new();
    let mut source = TrackedString::new("lvalue");
    let result = {
        let mut wrapped_mut_ref = by_mut_ref
            .wrap(|input| {
                input.push_suffix("!");
                input.clone()
            })
            .expect("mut-ref wrap");
        wrapped_mut_ref(&mut source)
    };
    assert_eq!(result.copies, 1);
    assert_eq!(result.moves, 0);
    assert_eq!(result.value, "lvalue!");
    assert_eq!(source.value, result.value);
}

#[test]
fn start_up_type_display_underlying_value_output() {
    assert_eq!(StartUpType::Fresh.to_string(), "0");
    assert_eq!(StartUpType::Network.to_string(), "5");
}

#[test]
fn fixed_network_id_service_read_only_contract() {
    let service = FixedNetworkIdService::new(1025);
    assert_eq!(service.get_network_id(), 1025);
}

#[test]
fn peer_reservation_table_insert_replace_and_erase_shape() {
    let table = PeerReservationTable::new();
    let first = PeerReservation::new(vec![1, 2, 3], "first");
    let replacement = PeerReservation::new(vec![1, 2, 3], "replacement");
    let other = PeerReservation::new(vec![9, 9, 9], "other");

    assert!(!table.contains(&first.node_id));
    assert!(table.insert_or_assign(first.clone()).is_none());
    assert!(table.contains(&first.node_id));
    assert_eq!(table.list(), vec![first.clone()]);

    assert_eq!(table.insert_or_assign(replacement.clone()), Some(first));
    assert_eq!(table.insert_or_assign(other.clone()), None);
    assert_eq!(table.list(), vec![replacement.clone(), other.clone()]);
    assert_eq!(table.erase(&replacement.node_id), Some(replacement));
    assert_eq!(table.erase(&vec![7]), None);
    assert_eq!(table.list(), vec![other]);
}

struct DummyRegistry;

static CONFIG_ID: usize = 11;
static OPEN_LEDGER_ID: usize = 7;
static WALLET_DB_ID: usize = 13;
static APP_ID: usize = 17;
static NONE_TRAP_TX_ID: Option<u64> = None;

fn assert_registry_surface<R>(registry: &R)
where
    R: ServiceRegistry<
            Journal = String,
            TrapTxId = u64,
            OpenLedger = usize,
            CollectorManager = (),
            NodeFamily = (),
            TimeKeeper = (),
            JobQueue = (),
            TempNodeCache = (),
            CachedSles = (),
            NetworkIdService = (),
            AmendmentTable = (),
            HashRouter = (),
            LoadFeeTrack = (),
            LoadManager = (),
            Validations = (),
            ValidatorList = (),
            ValidatorSite = (),
            ManifestCache = (),
            Overlay = (),
            Cluster = (),
            PeerReservationTable = (),
            ResourceManager = (),
            NodeStore = (),
            ShamapStore = (),
            RelationalDatabase = (),
            InboundLedgers = (),
            InboundTransactions = (),
            AcceptedLedgerCache = (),
            LedgerMaster = (),
            LedgerCleaner = (),
            LedgerReplayer = (),
            PendingSaves = (),
            NetworkOps = (),
            OrderBookDb = (),
            TransactionMaster = (),
            TxQ = (),
            PathRequestManager = (),
            ServerHandler = (),
            PerfLog = (),
            IoContext = (),
            Config = usize,
            Logs = (),
            WalletDb = usize,
            Application = usize,
        >,
{
    assert_registry_holder(registry);
}

fn assert_registry_holder<R>(registry: R)
where
    R: ServiceRegistry<
            Journal = String,
            TrapTxId = u64,
            OpenLedger = usize,
            CollectorManager = (),
            NodeFamily = (),
            TimeKeeper = (),
            JobQueue = (),
            TempNodeCache = (),
            CachedSles = (),
            NetworkIdService = (),
            AmendmentTable = (),
            HashRouter = (),
            LoadFeeTrack = (),
            LoadManager = (),
            Validations = (),
            ValidatorList = (),
            ValidatorSite = (),
            ManifestCache = (),
            Overlay = (),
            Cluster = (),
            PeerReservationTable = (),
            ResourceManager = (),
            NodeStore = (),
            ShamapStore = (),
            RelationalDatabase = (),
            InboundLedgers = (),
            InboundTransactions = (),
            AcceptedLedgerCache = (),
            LedgerMaster = (),
            LedgerCleaner = (),
            LedgerReplayer = (),
            PendingSaves = (),
            NetworkOps = (),
            OrderBookDb = (),
            TransactionMaster = (),
            TxQ = (),
            PathRequestManager = (),
            ServerHandler = (),
            PerfLog = (),
            IoContext = (),
            Config = usize,
            Logs = (),
            WalletDb = usize,
            Application = usize,
        >,
{
    assert!(!registry.is_stopping());
    assert_eq!(registry.get_journal("alpha"), "journal:alpha");
    assert_eq!(registry.get_journal("beta"), "journal:beta");
    assert_eq!(registry.get_config(), &CONFIG_ID);
    assert!(ptr::eq(
        registry.get_open_ledger(),
        registry.get_open_ledger_const()
    ));
    assert_eq!(*registry.get_open_ledger_const(), OPEN_LEDGER_ID);
    assert_eq!(registry.get_wallet_db(), &WALLET_DB_ID);
    assert_eq!(registry.get_app(), &APP_ID);
    assert_eq!(registry.get_trap_tx_id(), &NONE_TRAP_TX_ID);
}

impl ServiceRegistry for DummyRegistry {
    type CollectorManager = ();
    type NodeFamily = ();
    type TimeKeeper = ();
    type JobQueue = ();
    type TempNodeCache = ();
    type CachedSles = ();
    type NetworkIdService = ();
    type AmendmentTable = ();
    type HashRouter = ();
    type LoadFeeTrack = ();
    type LoadManager = ();
    type Validations = ();
    type ValidatorList = ();
    type ValidatorSite = ();
    type ManifestCache = ();
    type Overlay = ();
    type Cluster = ();
    type PeerReservationTable = ();
    type ResourceManager = ();
    type NodeStore = ();
    type ShamapStore = ();
    type RelationalDatabase = ();
    type InboundLedgers = ();
    type InboundTransactions = ();
    type AcceptedLedgerCache = ();
    type LedgerMaster = ();
    type LedgerCleaner = ();
    type LedgerReplayer = ();
    type PendingSaves = ();
    type OpenLedger = usize;
    type NetworkOps = ();
    type OrderBookDb = ();
    type TransactionMaster = ();
    type TxQ = ();
    type PathRequestManager = ();
    type ServerHandler = ();
    type PerfLog = ();
    type Journal = String;
    type IoContext = ();
    type Config = usize;
    type Logs = ();
    type TrapTxId = u64;
    type WalletDb = usize;
    type Application = usize;

    fn get_collector_manager(&self) -> &Self::CollectorManager {
        &()
    }
    fn get_node_family(&self) -> &Self::NodeFamily {
        &()
    }
    fn get_time_keeper(&self) -> &Self::TimeKeeper {
        &()
    }
    fn get_job_queue(&self) -> &Self::JobQueue {
        &()
    }
    fn get_temp_node_cache(&self) -> &Self::TempNodeCache {
        &()
    }
    fn get_cached_sles(&self) -> &Self::CachedSles {
        &()
    }
    fn get_network_id_service(&self) -> &Self::NetworkIdService {
        &()
    }
    fn get_amendment_table(&self) -> &Self::AmendmentTable {
        &()
    }
    fn get_hash_router(&self) -> &Self::HashRouter {
        &()
    }
    fn get_fee_track(&self) -> &Self::LoadFeeTrack {
        &()
    }
    fn get_load_manager(&self) -> &Self::LoadManager {
        &()
    }
    fn get_validations(&self) -> &Self::Validations {
        &()
    }
    fn get_validators(&self) -> &Self::ValidatorList {
        &()
    }
    fn get_validator_sites(&self) -> &Self::ValidatorSite {
        &()
    }
    fn get_validator_manifests(&self) -> &Self::ManifestCache {
        &()
    }
    fn get_publisher_manifests(&self) -> &Self::ManifestCache {
        &()
    }
    fn get_overlay(&self) -> &Self::Overlay {
        &()
    }
    fn get_cluster(&self) -> &Self::Cluster {
        &()
    }
    fn get_peer_reservations(&self) -> &Self::PeerReservationTable {
        &()
    }
    fn get_resource_manager(&self) -> &Self::ResourceManager {
        &()
    }
    fn get_node_store(&self) -> &Self::NodeStore {
        &()
    }
    fn get_shamap_store(&self) -> &Self::ShamapStore {
        &()
    }
    fn get_relational_database(&self) -> &Self::RelationalDatabase {
        &()
    }
    fn get_inbound_ledgers(&self) -> &Self::InboundLedgers {
        &()
    }
    fn get_inbound_transactions(&self) -> &Self::InboundTransactions {
        &()
    }
    fn get_accepted_ledger_cache(&self) -> &Self::AcceptedLedgerCache {
        &()
    }
    fn get_ledger_master(&self) -> &Self::LedgerMaster {
        &()
    }
    fn get_ledger_cleaner(&self) -> &Self::LedgerCleaner {
        &()
    }
    fn get_ledger_replayer(&self) -> &Self::LedgerReplayer {
        &()
    }
    fn get_pending_saves(&self) -> &Self::PendingSaves {
        &()
    }
    fn get_open_ledger(&self) -> &Self::OpenLedger {
        &OPEN_LEDGER_ID
    }
    fn get_ops(&self) -> &Self::NetworkOps {
        &()
    }
    fn get_order_book_db(&self) -> &Self::OrderBookDb {
        &()
    }
    fn get_master_transaction(&self) -> &Self::TransactionMaster {
        &()
    }
    fn get_tx_q(&self) -> &Self::TxQ {
        &()
    }
    fn get_path_request_manager(&self) -> &Self::PathRequestManager {
        &()
    }
    fn get_server_handler(&self) -> &Self::ServerHandler {
        &()
    }
    fn get_perf_log(&self) -> &Self::PerfLog {
        &()
    }
    fn is_stopping(&self) -> bool {
        false
    }
    fn get_journal(&self, name: &str) -> Self::Journal {
        format!("journal:{name}")
    }
    fn get_io_context(&self) -> &Self::IoContext {
        &()
    }
    fn get_config(&self) -> &Self::Config {
        &CONFIG_ID
    }
    fn get_logs(&self) -> &Self::Logs {
        &()
    }
    fn get_trap_tx_id(&self) -> &Option<Self::TrapTxId> {
        &NONE_TRAP_TX_ID
    }
    fn get_wallet_db(&self) -> &Self::WalletDb {
        &WALLET_DB_ID
    }
    fn get_app(&self) -> &Self::Application {
        &APP_ID
    }
}

#[test]
fn service_registry_trait_is_implementable_as_a_dependency_shell() {
    let registry = DummyRegistry;
    assert_registry_surface(&registry);
}

#[test]
fn service_registry_forwarding_supports_box_and_arc_holders() {
    let boxed = Box::new(DummyRegistry);
    assert_registry_surface(&boxed);

    let shared = Arc::new(DummyRegistry);
    assert_registry_surface(&shared);

    let local_shared = Rc::new(DummyRegistry);
    assert_registry_surface(&local_shared);

    let borrowed = &DummyRegistry;
    assert_registry_surface(&borrowed);

    let mut mutable = DummyRegistry;
    assert_registry_holder(&mut mutable);
}

#[test]
fn config_sections_match_current_cpp_literals() {
    assert_eq!(ConfigSection::node_database(), "node_db");
    assert_eq!(ConfigSection::import_node_database(), "import_db");
    assert_eq!(SECTION_PORT_GRPC, "port_grpc");
    assert_eq!(SECTION_NODE_SIZE, "node_size");
    assert_eq!(SECTION_SERVER_DOMAIN, "server_domain");
}
