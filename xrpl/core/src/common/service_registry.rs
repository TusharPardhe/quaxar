use std::rc::Rc;
use std::sync::Arc;

pub trait ServiceRegistry {
    type CollectorManager;
    type NodeFamily;
    type TimeKeeper;
    type JobQueue;
    type TempNodeCache;
    type CachedSles;
    type NetworkIdService;
    type AmendmentTable;
    type HashRouter;
    type LoadFeeTrack;
    type LoadManager;
    type Validations;
    type ValidatorList;
    type ValidatorSite;
    type ManifestCache;
    type Overlay;
    type Cluster;
    type PeerReservationTable;
    type ResourceManager;
    type NodeStore;
    type ShamapStore;
    type RelationalDatabase;
    type InboundLedgers;
    type InboundTransactions;
    type AcceptedLedgerCache;
    type LedgerMaster;
    type LedgerCleaner;
    type LedgerReplayer;
    type PendingSaves;
    type OpenLedger;
    type NetworkOps;
    type OrderBookDb;
    type TransactionMaster;
    type TxQ;
    type PathRequestManager;
    type ServerHandler;
    type PerfLog;
    type Journal;
    type IoContext;
    type Config;
    type Logs;
    type TrapTxId;
    type WalletDb;
    type Application;

    fn get_collector_manager(&self) -> &Self::CollectorManager;
    fn get_node_family(&self) -> &Self::NodeFamily;
    fn get_time_keeper(&self) -> &Self::TimeKeeper;
    fn get_job_queue(&self) -> &Self::JobQueue;
    fn get_temp_node_cache(&self) -> &Self::TempNodeCache;
    fn get_cached_sles(&self) -> &Self::CachedSles;
    fn get_network_id_service(&self) -> &Self::NetworkIdService;
    fn get_amendment_table(&self) -> &Self::AmendmentTable;
    fn get_hash_router(&self) -> &Self::HashRouter;
    fn get_fee_track(&self) -> &Self::LoadFeeTrack;
    fn get_load_manager(&self) -> &Self::LoadManager;
    fn get_validations(&self) -> &Self::Validations;
    fn get_validators(&self) -> &Self::ValidatorList;
    fn get_validator_sites(&self) -> &Self::ValidatorSite;
    fn get_validator_manifests(&self) -> &Self::ManifestCache;
    fn get_publisher_manifests(&self) -> &Self::ManifestCache;
    fn get_overlay(&self) -> &Self::Overlay;
    fn get_cluster(&self) -> &Self::Cluster;
    fn get_peer_reservations(&self) -> &Self::PeerReservationTable;
    fn get_resource_manager(&self) -> &Self::ResourceManager;
    fn get_node_store(&self) -> &Self::NodeStore;
    fn get_shamap_store(&self) -> &Self::ShamapStore;
    fn get_relational_database(&self) -> &Self::RelationalDatabase;
    fn get_inbound_ledgers(&self) -> &Self::InboundLedgers;
    fn get_inbound_transactions(&self) -> &Self::InboundTransactions;
    fn get_accepted_ledger_cache(&self) -> &Self::AcceptedLedgerCache;
    fn get_ledger_master(&self) -> &Self::LedgerMaster;
    fn get_ledger_cleaner(&self) -> &Self::LedgerCleaner;
    fn get_ledger_replayer(&self) -> &Self::LedgerReplayer;
    fn get_pending_saves(&self) -> &Self::PendingSaves;
    fn get_open_ledger(&self) -> &Self::OpenLedger;
    fn get_open_ledger_const(&self) -> &Self::OpenLedger {
        self.get_open_ledger()
    }
    fn get_ops(&self) -> &Self::NetworkOps;
    fn get_order_book_db(&self) -> &Self::OrderBookDb;
    fn get_master_transaction(&self) -> &Self::TransactionMaster;
    fn get_tx_q(&self) -> &Self::TxQ;
    fn get_path_request_manager(&self) -> &Self::PathRequestManager;
    fn get_server_handler(&self) -> &Self::ServerHandler;
    fn get_perf_log(&self) -> &Self::PerfLog;
    fn is_stopping(&self) -> bool;
    fn get_journal(&self, name: &str) -> Self::Journal;
    fn get_io_context(&self) -> &Self::IoContext;
    fn get_config(&self) -> &Self::Config;
    fn get_logs(&self) -> &Self::Logs;
    fn get_trap_tx_id(&self) -> &Option<Self::TrapTxId>;
    fn get_wallet_db(&self) -> &Self::WalletDb;
    fn get_app(&self) -> &Self::Application;
}

macro_rules! impl_service_registry_forwarding {
    ($wrapper:ty, $unwrap:ident) => {
        impl<T> ServiceRegistry for $wrapper
        where
            T: ServiceRegistry + ?Sized,
        {
            type CollectorManager = T::CollectorManager;
            type NodeFamily = T::NodeFamily;
            type TimeKeeper = T::TimeKeeper;
            type JobQueue = T::JobQueue;
            type TempNodeCache = T::TempNodeCache;
            type CachedSles = T::CachedSles;
            type NetworkIdService = T::NetworkIdService;
            type AmendmentTable = T::AmendmentTable;
            type HashRouter = T::HashRouter;
            type LoadFeeTrack = T::LoadFeeTrack;
            type LoadManager = T::LoadManager;
            type Validations = T::Validations;
            type ValidatorList = T::ValidatorList;
            type ValidatorSite = T::ValidatorSite;
            type ManifestCache = T::ManifestCache;
            type Overlay = T::Overlay;
            type Cluster = T::Cluster;
            type PeerReservationTable = T::PeerReservationTable;
            type ResourceManager = T::ResourceManager;
            type NodeStore = T::NodeStore;
            type ShamapStore = T::ShamapStore;
            type RelationalDatabase = T::RelationalDatabase;
            type InboundLedgers = T::InboundLedgers;
            type InboundTransactions = T::InboundTransactions;
            type AcceptedLedgerCache = T::AcceptedLedgerCache;
            type LedgerMaster = T::LedgerMaster;
            type LedgerCleaner = T::LedgerCleaner;
            type LedgerReplayer = T::LedgerReplayer;
            type PendingSaves = T::PendingSaves;
            type OpenLedger = T::OpenLedger;
            type NetworkOps = T::NetworkOps;
            type OrderBookDb = T::OrderBookDb;
            type TransactionMaster = T::TransactionMaster;
            type TxQ = T::TxQ;
            type PathRequestManager = T::PathRequestManager;
            type ServerHandler = T::ServerHandler;
            type PerfLog = T::PerfLog;
            type Journal = T::Journal;
            type IoContext = T::IoContext;
            type Config = T::Config;
            type Logs = T::Logs;
            type TrapTxId = T::TrapTxId;
            type WalletDb = T::WalletDb;
            type Application = T::Application;

            fn get_collector_manager(&self) -> &Self::CollectorManager {
                let target = $unwrap(self);
                target.get_collector_manager()
            }
            fn get_node_family(&self) -> &Self::NodeFamily {
                let target = $unwrap(self);
                target.get_node_family()
            }
            fn get_time_keeper(&self) -> &Self::TimeKeeper {
                let target = $unwrap(self);
                target.get_time_keeper()
            }
            fn get_job_queue(&self) -> &Self::JobQueue {
                let target = $unwrap(self);
                target.get_job_queue()
            }
            fn get_temp_node_cache(&self) -> &Self::TempNodeCache {
                let target = $unwrap(self);
                target.get_temp_node_cache()
            }
            fn get_cached_sles(&self) -> &Self::CachedSles {
                let target = $unwrap(self);
                target.get_cached_sles()
            }
            fn get_network_id_service(&self) -> &Self::NetworkIdService {
                let target = $unwrap(self);
                target.get_network_id_service()
            }
            fn get_amendment_table(&self) -> &Self::AmendmentTable {
                let target = $unwrap(self);
                target.get_amendment_table()
            }
            fn get_hash_router(&self) -> &Self::HashRouter {
                let target = $unwrap(self);
                target.get_hash_router()
            }
            fn get_fee_track(&self) -> &Self::LoadFeeTrack {
                let target = $unwrap(self);
                target.get_fee_track()
            }
            fn get_load_manager(&self) -> &Self::LoadManager {
                let target = $unwrap(self);
                target.get_load_manager()
            }
            fn get_validations(&self) -> &Self::Validations {
                let target = $unwrap(self);
                target.get_validations()
            }
            fn get_validators(&self) -> &Self::ValidatorList {
                let target = $unwrap(self);
                target.get_validators()
            }
            fn get_validator_sites(&self) -> &Self::ValidatorSite {
                let target = $unwrap(self);
                target.get_validator_sites()
            }
            fn get_validator_manifests(&self) -> &Self::ManifestCache {
                let target = $unwrap(self);
                target.get_validator_manifests()
            }
            fn get_publisher_manifests(&self) -> &Self::ManifestCache {
                let target = $unwrap(self);
                target.get_publisher_manifests()
            }
            fn get_overlay(&self) -> &Self::Overlay {
                let target = $unwrap(self);
                target.get_overlay()
            }
            fn get_cluster(&self) -> &Self::Cluster {
                let target = $unwrap(self);
                target.get_cluster()
            }
            fn get_peer_reservations(&self) -> &Self::PeerReservationTable {
                let target = $unwrap(self);
                target.get_peer_reservations()
            }
            fn get_resource_manager(&self) -> &Self::ResourceManager {
                let target = $unwrap(self);
                target.get_resource_manager()
            }
            fn get_node_store(&self) -> &Self::NodeStore {
                let target = $unwrap(self);
                target.get_node_store()
            }
            fn get_shamap_store(&self) -> &Self::ShamapStore {
                let target = $unwrap(self);
                target.get_shamap_store()
            }
            fn get_relational_database(&self) -> &Self::RelationalDatabase {
                let target = $unwrap(self);
                target.get_relational_database()
            }
            fn get_inbound_ledgers(&self) -> &Self::InboundLedgers {
                let target = $unwrap(self);
                target.get_inbound_ledgers()
            }
            fn get_inbound_transactions(&self) -> &Self::InboundTransactions {
                let target = $unwrap(self);
                target.get_inbound_transactions()
            }
            fn get_accepted_ledger_cache(&self) -> &Self::AcceptedLedgerCache {
                let target = $unwrap(self);
                target.get_accepted_ledger_cache()
            }
            fn get_ledger_master(&self) -> &Self::LedgerMaster {
                let target = $unwrap(self);
                target.get_ledger_master()
            }
            fn get_ledger_cleaner(&self) -> &Self::LedgerCleaner {
                let target = $unwrap(self);
                target.get_ledger_cleaner()
            }
            fn get_ledger_replayer(&self) -> &Self::LedgerReplayer {
                let target = $unwrap(self);
                target.get_ledger_replayer()
            }
            fn get_pending_saves(&self) -> &Self::PendingSaves {
                let target = $unwrap(self);
                target.get_pending_saves()
            }
            fn get_open_ledger(&self) -> &Self::OpenLedger {
                let target = $unwrap(self);
                target.get_open_ledger()
            }
            fn get_open_ledger_const(&self) -> &Self::OpenLedger {
                let target = $unwrap(self);
                target.get_open_ledger_const()
            }
            fn get_ops(&self) -> &Self::NetworkOps {
                let target = $unwrap(self);
                target.get_ops()
            }
            fn get_order_book_db(&self) -> &Self::OrderBookDb {
                let target = $unwrap(self);
                target.get_order_book_db()
            }
            fn get_master_transaction(&self) -> &Self::TransactionMaster {
                let target = $unwrap(self);
                target.get_master_transaction()
            }
            fn get_tx_q(&self) -> &Self::TxQ {
                let target = $unwrap(self);
                target.get_tx_q()
            }
            fn get_path_request_manager(&self) -> &Self::PathRequestManager {
                let target = $unwrap(self);
                target.get_path_request_manager()
            }
            fn get_server_handler(&self) -> &Self::ServerHandler {
                let target = $unwrap(self);
                target.get_server_handler()
            }
            fn get_perf_log(&self) -> &Self::PerfLog {
                let target = $unwrap(self);
                target.get_perf_log()
            }
            fn is_stopping(&self) -> bool {
                let target = $unwrap(self);
                target.is_stopping()
            }
            fn get_journal(&self, name: &str) -> Self::Journal {
                let target = $unwrap(self);
                target.get_journal(name)
            }
            fn get_io_context(&self) -> &Self::IoContext {
                let target = $unwrap(self);
                target.get_io_context()
            }
            fn get_config(&self) -> &Self::Config {
                let target = $unwrap(self);
                target.get_config()
            }
            fn get_logs(&self) -> &Self::Logs {
                let target = $unwrap(self);
                target.get_logs()
            }
            fn get_trap_tx_id(&self) -> &Option<Self::TrapTxId> {
                let target = $unwrap(self);
                target.get_trap_tx_id()
            }
            fn get_wallet_db(&self) -> &Self::WalletDb {
                let target = $unwrap(self);
                target.get_wallet_db()
            }
            fn get_app(&self) -> &Self::Application {
                let target = $unwrap(self);
                target.get_app()
            }
        }
    };
}

fn unwrap_borrowed<'a, T: ?Sized>(target: &'a &'a T) -> &'a T {
    target
}

fn unwrap_mut_borrowed<'a, T: ?Sized>(target: &'a &mut T) -> &'a T {
    target
}

#[allow(clippy::borrowed_box)]
fn unwrap_boxed<T: ?Sized>(target: &Box<T>) -> &T {
    target.as_ref()
}

fn unwrap_arc<T: ?Sized>(target: &Arc<T>) -> &T {
    target.as_ref()
}

fn unwrap_rc<T: ?Sized>(target: &Rc<T>) -> &T {
    target.as_ref()
}

impl_service_registry_forwarding!(&T, unwrap_borrowed);
impl_service_registry_forwarding!(&mut T, unwrap_mut_borrowed);
impl_service_registry_forwarding!(Box<T>, unwrap_boxed);
impl_service_registry_forwarding!(Arc<T>, unwrap_arc);
impl_service_registry_forwarding!(Rc<T>, unwrap_rc);
