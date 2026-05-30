//! Honest application-root owner for the migrated runtime shell.

use crate::amendments::amendment_status::{AmendmentStatus, UnsupportedMajorityWarningDetails};
use crate::consensus::rcl_validations::SharedAppValidations;
use crate::job::job_queue::JobQueue;
use crate::ledger::ledger_master_runtime::AppLedgerMasterRuntime;
use crate::ledger::ledger_master_state::SharedLedgerMasterState;
use crate::ledger::open_ledger::OpenLedgerView as _;
use crate::load::load_fee_track::SharedLoadFeeTrack;
use crate::load::load_manager::{LoadManager, LoadManagerTiming};
use crate::network::network_ops::networkops_apply_flags;
use crate::network::network_ops::{
    AppNetworkOpsModeOwner, NetworkOpsOperatingMode, SharedNetworkOpsState,
    normalize_operating_mode_for_validated_age,
};
use crate::network::network_ops_runtime::{
    AppNetworkOpsApplyHeldOutcome, AppNetworkOpsApplyReport, AppNetworkOpsRuntime,
    AppNetworkOpsSubmitReport,
};
use crate::network::network_ops_validation_runtime::{
    AppNetworkOpsValidationReceiveReport, AppNetworkOpsValidationRuntime,
};
use crate::node_family::node_family::{NodeFamily, NodeFamilyRuntime};
use crate::runtime::component_runtime::{
    AppConsensusRuntime, AppLedgerRuntime, AppNodeStoreRuntime, AppPerfLogRuntime,
    AppValidatorSiteRuntime,
};
use crate::runtime::main_runtime::{GrpcRuntime, ManagedComponent, ManagedHandle, RuntimeBindings};
use crate::runtime::overlay_runtime::{AppOverlayRuntime, build_overlay_runtime};
use crate::runtime::resolver_runtime::AppResolverRuntime;
use crate::server::server_okay::server_okay;
use crate::server::server_ports::{
    PublishedServerPortsSource, ServerPortsSetup, build_server_ports_setup,
};
use crate::shamap::shamap_store_component::SHAMapStoreComponent;
use crate::shamap::shamap_store_health::SHAMapStoreOperatingMode;
use crate::shamap::shamap_store_service::SHAMapStoreService;
use crate::state::accept_ledger_pending_apply::AcceptLedgerPendingApplyRuntime;
use crate::state::app_registry::{
    AppAcceptedLedgerCache, AppConfig, AppInboundLedgers, AppInboundTransactions, AppLogs,
    AppOpenLedgerTxRecord, AppOpenLedgerView, AppPlaceholder, AppQueueApplyTxSource,
    AppServerHandler, AppTxQAccount, AppTxQJournalTag, AppTxQLock, AppTxQParentBatchId,
    AppTxQTransaction, ApplicationRegistryOwners, SharedAppOpenLedger, SharedAppTxQ,
};
use crate::state::basic_app::BasicApp;
use crate::state::collector_manager::{CollectorManager, CollectorParams};
use crate::state::manifest::ManifestCache;
use crate::state::node_store_scheduler::NodeStoreScheduler;
use crate::state::overlay_status::OverlayStatusSource;
use crate::state::status_metrics::StatusMetricsSource;
use crate::state::status_rpc_state::{StatusRpcGitInfo, StatusRpcLastClose, StatusRpcState};
use crate::state::stop_tree::{StopTree, StopTreeNode};
use crate::state::time_keeper::{SystemTimeKeeperClock, TimeKeeper};
use crate::state::transactor_dispatcher::handle_real_dispatch;
use crate::tx_queue::transaction::{Transaction, TransactionCloseTimeSource};
use crate::tx_queue::transaction_master::{SharedTransaction, TransactionMaster};
use crate::validator::validator_list::{
    SystemValidatorListClock, ValidatorList, ValidatorListStatusSnapshot,
};
use crate::validator::validator_site::ValidatorSite;
use basics::base_uint::{Uint160, Uint256};
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::MonotonicClock;
use ledger::OrderBookDB;
use ledger::{
    CanonicalTXSet, Ledger, LedgerMasterCaughtUp, LedgerNodeObjectType, OpenView, ReadView,
    Sandbox, TxsRawView,
};
use overlay::Cluster;
use overlay::{OverlayHandoff, OverlayImpl, PeerReservationSource};
use perflog::PerfLogImp;
use protocol::{
    AccountID, BatchTransactionFlags, JsonOptions, JsonValue, NotTec, PublicKey, Rules, STAmount,
    STLedgerEntry, STTx, SecretKey, SeqProxy, Serializer, Ter, TxType, XRPAmount, account_keylet,
    feature_xrp_fees, get_field_by_symbol, is_tec_claim, is_tes_success,
};
use shamap::family::{NullFullBelowCache, NullMissingNodeReporter, NullNodeFetcher, SHAMapFamily};
use shamap::tree_node_cache::TreeNodeCache;
use std::sync::{Arc, Mutex};
use time::{Duration, OffsetDateTime};
use tx::{
    ApplyFlags, ApplyResult, HasTxnType, PreclaimResult, PreflightResult,
    QueueAcceptLedgerViewSource, QueueAcceptLiveApplyRuntime, QueueApplyExecutionRuntime,
    QueueFeeLevelPaidInputs, QueueTxQClosedLedgerAppSource, QueueTxQClosedLedgerView,
    QueueTxQMetrics, QueueTxQRpcReport, TxConsequences, TxDetails, evaluate_fee_level_paid,
    snapshot_queue_apply_app_view_with_metrics,
};
use xrpl_core::{
    FixedNetworkIdService, HashRouter, LoadMonitorJournalFactory, NetworkIDService,
    PeerReservationTable, ServiceRegistry, StartUpType,
};
use xrpld_core::DatabaseCon;

fn to_nodestore_type(object_type: LedgerNodeObjectType) -> nodestore::NodeObjectType {
    match object_type {
        LedgerNodeObjectType::AccountNode => nodestore::NodeObjectType::AccountNode,
        LedgerNodeObjectType::TransactionNode => nodestore::NodeObjectType::TransactionNode,
    }
}

fn full_sync_debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("XRPLD_FULL_SYNC_DEBUG")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    })
}

macro_rules! full_sync_debug {
    ($($arg:tt)*) => {
        if crate::state::application_root::full_sync_debug_enabled() {
            tracing::debug!(target: "full_sync", $($arg)*);
        }
    };
}

#[derive(Clone)]
struct AppLoadManagerEvents {
    collector_manager: CollectorManager,
}

impl crate::load::load_manager::LoadManagerEvents for AppLoadManagerEvents {
    fn report_fee_change(&self) {
        self.collector_manager
            .group("load_manager")
            .record_event("fee_change");
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationRootOptions {
    pub io_threads: usize,
    pub job_queue_threads: usize,
    pub start_valid: bool,
    pub elb_support: bool,
    pub standalone: bool,
    pub start_type: StartUpType,
    pub start_ledger: Option<String>,
    pub import: bool,
    pub quorum: Option<usize>,
    pub collector_params: CollectorParams,
    pub load_manager_timing: LoadManagerTiming,
}

impl Default for ApplicationRootOptions {
    fn default() -> Self {
        Self {
            io_threads: 1,
            job_queue_threads: 1,
            start_valid: false,
            elb_support: false,
            standalone: false,
            start_type: StartUpType::Fresh,
            start_ledger: None,
            import: false,
            quorum: None,
            collector_params: CollectorParams::default(),
            load_manager_timing: LoadManagerTiming::default(),
        }
    }
}

#[derive(Clone)]
pub struct ApplicationRoot {
    registry: ApplicationRegistryOwners,
    basic_app: Arc<BasicApp>,
    job_queue: Arc<JobQueue>,
    time_keeper: Arc<TimeKeeper<SystemTimeKeeperClock>>,
    stop_tree: Arc<StopTree>,
    collector_manager: Arc<CollectorManager>,
    load_manager: Arc<LoadManager>,
    load_fee_track: Arc<SharedLoadFeeTrack>,
    node_store_scheduler: Arc<NodeStoreScheduler>,
    node_family: Option<Arc<dyn NodeFamilyRuntime>>,
    resolver_runtime: Option<Arc<AppResolverRuntime>>,
    overlay_runtime: Option<Arc<AppOverlayRuntime>>,
    overlay_status: Option<Arc<dyn OverlayStatusSource>>,
    server_ports_setup: Option<Arc<ServerPortsSetup>>,
    published_server_ports: Option<Arc<dyn PublishedServerPortsSource>>,
    status_metrics: Option<Arc<dyn StatusMetricsSource>>,
    network_ops_state: Arc<SharedNetworkOpsState>,
    network_ops_runtime: Option<Arc<AppNetworkOpsRuntime>>,
    network_ops_validation_runtime: Option<Arc<AppNetworkOpsValidationRuntime>>,
    ledger_master_runtime: Option<Arc<AppLedgerMasterRuntime>>,
    consensus_runtime: Option<Arc<AppConsensusRuntime>>,
    ledger_master_state: Arc<SharedLedgerMasterState>,
    transaction_master: Arc<TransactionMaster>,
    validations: SharedAppValidations<SystemTimeKeeperClock>,
    validators: Arc<ValidatorList>,
    status_rpc_state: Arc<StatusRpcState>,
    amendment_status: Arc<AmendmentStatus>,
    elb_support: bool,
    node_identity: Option<(PublicKey, SecretKey)>,
    validation_public_key: Option<PublicKey>,
    runtime_bindings: RuntimeBindings,
    shamap_store_service: Option<Arc<SHAMapStoreService>>,
    /// Shared node store for ConsensusLedgerAcceptor. Populated by attach_node_store.
    shared_consensus_node_store:
        Arc<std::sync::RwLock<Option<crate::shamap::shamap_store_backend::SHAMapStoreNodeStore>>>,
}

impl std::fmt::Debug for ApplicationRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApplicationRoot")
            .field("basic_app", &self.basic_app)
            .field("job_queue", &self.job_queue)
            .field("collector_manager", &self.collector_manager)
            .field("load_manager", &self.load_manager)
            .field("local_load_fee", &self.load_fee_track.local_fee())
            .field("node_store_scheduler", &self.node_store_scheduler)
            .field("time_keeper", &"TimeKeeper")
            .field("stop_tree", &self.stop_tree)
            .field("has_node_family", &self.node_family.is_some())
            .field("has_resolver_runtime", &self.resolver_runtime.is_some())
            .field("has_overlay_runtime", &self.overlay_runtime.is_some())
            .field("has_overlay_status", &self.overlay_status.is_some())
            .field("has_server_ports_setup", &self.server_ports_setup.is_some())
            .field(
                "has_published_server_ports",
                &self.published_server_ports.is_some(),
            )
            .field("has_status_metrics", &self.status_metrics().is_some())
            .field("wallet_db_path", &self.registry.config.wallet_db_path)
            .field("path_search_old", &self.registry.config.path_search_old)
            .field("path_search", &self.registry.config.path_search)
            .field("path_search_fast", &self.registry.config.path_search_fast)
            .field("path_search_max", &self.registry.config.path_search_max)
            .field(
                "peer_reservation_count",
                &self.registry.peer_reservations.list().len(),
            )
            .field("has_perf_log", &self.registry.perf_log.is_some())
            .field(
                "network_ops_operating_mode",
                &self.network_ops_operating_mode(),
            )
            .field(
                "has_network_ops_runtime",
                &self.network_ops_runtime.is_some(),
            )
            .field(
                "has_network_ops_validation_runtime",
                &self.network_ops_validation_runtime.is_some(),
            )
            .field(
                "network_ops_pending_transactions",
                &self.network_ops_pending_transaction_count().unwrap_or(0),
            )
            .field(
                "network_ops_pending_validations",
                &self.network_ops_pending_validation_count().unwrap_or(0),
            )
            .field(
                "transaction_cache_size",
                &self.transaction_master.get_cache().size(),
            )
            .field("validated_ledger_seq", &self.validated_ledger_seq())
            .field("published_ledger_seq", &self.published_ledger_seq())
            .field(
                "has_ledger_master_runtime",
                &self.ledger_master_runtime.is_some(),
            )
            .field("has_consensus_runtime", &self.consensus_runtime.is_some())
            .field("local_tx_count", &self.local_tx_count().unwrap_or(0))
            .field("validations", &self.validations)
            .field("validator_quorum", &self.validators.quorum())
            .field("validator_list_count", &self.validators.count())
            .field(
                "status_rpc_current_ledger_index",
                &self.status_rpc_current_ledger_index(),
            )
            .field(
                "has_status_rpc_queue_report",
                &self.status_rpc_queue_report().is_some(),
            )
            .field("status_rpc_peer_count", &self.status_rpc_peer_count())
            .field("status_rpc_network_id", &self.status_rpc_network_id())
            .field(
                "has_status_rpc_last_close",
                &self.status_rpc_last_close().is_some(),
            )
            .field("status_rpc_hostid", &self.status_rpc_hostid())
            .field("status_rpc_server_domain", &self.status_rpc_server_domain())
            .field("status_rpc_node_size", &self.status_rpc_node_size())
            .field("status_rpc_io_latency_ms", &self.status_rpc_io_latency_ms())
            .field(
                "has_status_rpc_git_info",
                &self.status_rpc_git_info().is_some(),
            )
            .field(
                "unsupported_majority_warned",
                &self.unsupported_majority_warned(),
            )
            .field(
                "has_unsupported_majority_warning_details",
                &self.unsupported_majority_warning_details().is_some(),
            )
            .field("elb_support", &self.elb_support)
            .field("has_node_identity", &self.node_identity.is_some())
            .field(
                "has_validation_public_key",
                &self.validation_public_key.is_some(),
            )
            .field("runtime_bindings", &self.runtime_bindings())
            .field(
                "has_shamap_store_service",
                &self.shamap_store_service.is_some(),
            )
            .finish()
    }
}

pub trait LedgerAcceptor: Send + Sync + 'static {
    fn accept_ledger(
        &self,
        closed_seq: u32,
        close_time: u32,
        base_fee_drops: u64,
    ) -> Result<u32, String>;

    /// Accept a ledger built by the consensus engine.
    fn consensus_built(&self, ledger: Arc<Ledger>) -> Result<(), String>;

    /// Return the owner-tracked closed ledger for consensus handoff.
    fn consensus_closed_ledger(&self) -> Option<Arc<Ledger>> {
        None
    }

    /// Return the owner-selected previous ledger for the next round.
    fn consensus_previous_ledger(&self) -> Option<Arc<Ledger>> {
        None
    }

    /// Get a node fetcher closure for backed state map reads from NuDB.
    fn node_fetcher(
        &self,
    ) -> Option<
        Arc<
            dyn Fn(
                    basics::sha_map_hash::SHAMapHash,
                ) -> Option<
                    basics::memory::intrusive_pointer::SharedIntrusive<
                        shamap::nodes::tree_node::SHAMapTreeNode,
                    >,
                > + Send
                + Sync,
        >,
    > {
        None
    }
}

impl LedgerAcceptor for ApplicationRoot {
    fn accept_ledger(
        &self,
        closed_seq: u32,
        close_time: u32,
        base_fee_drops: u64,
    ) -> Result<u32, String> {
        self.accept_ledger(closed_seq, close_time, base_fee_drops)
    }

    fn consensus_built(&self, ledger: Arc<ledger::Ledger>) -> Result<(), String> {
        tracing::info!(target: "consensus",
            "[consensus] consensus_built seq={} hash={:02x}{:02x}{:02x}{:02x}",
            ledger.header().seq,
            ledger.header().hash.as_uint256().data()[0],
            ledger.header().hash.as_uint256().data()[1],
            ledger.header().hash.as_uint256().data()[2],
            ledger.header().hash.as_uint256().data()[3],
        );
        self.on_consensus_built_ledger(ledger);
        Ok(())
    }

    fn consensus_closed_ledger(&self) -> Option<Arc<Ledger>> {
        self.closed_ledger().or_else(|| self.validated_ledger())
    }

    fn consensus_previous_ledger(&self) -> Option<Arc<Ledger>> {
        let parent_hash = self.open_ledger().current().parent_hash;
        if parent_hash.is_zero() {
            return self.consensus_closed_ledger();
        }

        if let Some(closed) = self.consensus_closed_ledger()
            && *closed.header().hash.as_uint256() == parent_hash
        {
            return Some(closed);
        }

        self.ledger_master_runtime()
            .and_then(|runtime| {
                runtime
                    .ledger_master()
                    .get_ledger_by_hash(SHAMapHash::new(parent_hash))
            })
            .or_else(|| {
                self.validated_ledger()
                    .filter(|ledger| *ledger.header().hash.as_uint256() == parent_hash)
            })
    }

    fn node_fetcher(
        &self,
    ) -> Option<
        Arc<
            dyn Fn(
                    basics::sha_map_hash::SHAMapHash,
                ) -> Option<
                    basics::memory::intrusive_pointer::SharedIntrusive<
                        shamap::nodes::tree_node::SHAMapTreeNode,
                    >,
                > + Send
                + Sync,
        >,
    > {
        let ns = self.node_store().as_ref()?.clone();
        Some(Arc::new(move |hash| {
            let data = match &ns {
                crate::shamap::shamap_store_backend::SHAMapStoreNodeStore::Single(db) => db
                    .fetch_node_object(
                        hash.as_uint256(),
                        0,
                        nodestore::FetchType::Synchronous,
                        false,
                    ),
                crate::shamap::shamap_store_backend::SHAMapStoreNodeStore::Rotating(db) => db
                    .fetch_node_object(
                        hash.as_uint256(),
                        0,
                        nodestore::FetchType::Synchronous,
                        false,
                    ),
            }?;
            shamap::nodes::tree_node::SHAMapTreeNode::make_from_prefix(data.data(), hash).ok()
        }))
    }
}

#[allow(dead_code)]
pub struct ConsensusLedgerAcceptor {
    root: ApplicationRoot,
    job_queue: Arc<JobQueue>,
    basic_app: Arc<BasicApp>,
    /// Shared node store reference. Set via OnceLock after node store is attached.
    /// This avoids the initialization order issue where the consensus runtime is
    /// created before the node store is attached to ApplicationRoot.
    shared_node_store:
        Arc<std::sync::RwLock<Option<crate::shamap::shamap_store_backend::SHAMapStoreNodeStore>>>,
}

#[derive(Clone)]
struct AcceptLedgerPendingTransaction {
    transaction: SharedTransaction,
}

impl HasTxnType for AcceptLedgerPendingTransaction {
    fn txn_type(&self) -> TxType {
        self.transaction
            .lock()
            .expect("transaction mutex must not be poisoned")
            .get_s_transaction()
            .get_txn_type()
    }
}

struct AcceptLedgerPendingRuntime;

#[derive(Clone, Copy)]
struct QueueApplyPreclaimTx<'a> {
    tx: &'a STTx,
}

#[derive(Debug, Clone)]
struct SubmitConsumedTicket {
    sle: Arc<STLedgerEntry>,
    owner_page: u64,
}

impl HasTxnType for QueueApplyPreclaimTx<'_> {
    fn txn_type(&self) -> TxType {
        self.tx.get_txn_type()
    }
}

impl tx::TransactorCheckSeqProxyTx for QueueApplyPreclaimTx<'_> {
    type AccountId = AccountID;

    fn account_id(&self) -> Self::AccountId {
        self.tx.get_account_id(get_field_by_symbol("sfAccount"))
    }

    fn seq_proxy(&self) -> SeqProxy {
        self.tx.get_seq_proxy()
    }

    fn ticket_sequence_present(&self) -> bool {
        self.tx
            .is_field_present(get_field_by_symbol("sfTicketSequence"))
    }
}

impl tx::TransactorCheckPriorTxAndLastLedgerTx for QueueApplyPreclaimTx<'_> {
    type AccountId = AccountID;
    type TxId = Uint256;

    fn account_id(&self) -> Self::AccountId {
        self.tx.get_account_id(get_field_by_symbol("sfAccount"))
    }

    fn account_txn_id(&self) -> Option<Self::TxId> {
        self.tx
            .is_field_present(get_field_by_symbol("sfAccountTxnID"))
            .then(|| {
                self.tx
                    .get_field_h256(get_field_by_symbol("sfAccountTxnID"))
            })
    }

    fn last_ledger_sequence(&self) -> Option<u32> {
        self.tx
            .is_field_present(get_field_by_symbol("sfLastLedgerSequence"))
            .then(|| {
                self.tx
                    .get_field_u32(get_field_by_symbol("sfLastLedgerSequence"))
            })
    }

    fn transaction_id(&self) -> Self::TxId {
        self.tx.get_transaction_id()
    }
}

#[derive(Clone, Copy)]
struct AppClosedLedgerTxQView<'a> {
    ledger: &'a Ledger,
}

impl QueueTxQClosedLedgerView for AppClosedLedgerTxQView<'_> {
    fn ledger_seq(&self) -> u32 {
        self.ledger.header().seq
    }
}

#[derive(Clone, Copy)]
struct AppOpenLedgerTxQAcceptView {
    open_ledger_tx_count: usize,
    parent_hash: Uint256,
}

impl QueueAcceptLedgerViewSource for AppOpenLedgerTxQAcceptView {
    fn open_ledger_tx_count(&self) -> usize {
        self.open_ledger_tx_count
    }

    fn parent_hash(&self) -> Uint256 {
        self.parent_hash
    }
}

struct AppOpenLedgerTxQAcceptRuntime<'a> {
    view: &'a mut AppOpenLedgerView,
}

impl
    QueueAcceptLiveApplyRuntime<
        AppTxQAccount,
        AppTxQTransaction,
        AppTxQJournalTag,
        AppTxQParentBatchId,
    > for AppOpenLedgerTxQAcceptRuntime<'_>
{
    fn apply_queued(
        &mut self,
        queued: &mut tx::MaybeTx<
            AppTxQTransaction,
            AppTxQAccount,
            AppTxQJournalTag,
            AppTxQParentBatchId,
        >,
    ) -> ApplyResult {
        self.view.push_transaction(Arc::clone(&queued.pf_result.tx));
        ApplyResult::new(Ter::TES_SUCCESS, true, false)
    }
}

struct AppOpenLedgerTxQApplyRuntime<'a> {
    view: &'a mut AppOpenLedgerView,
    submit_view: &'a mut Sandbox<Ledger>,
    tx: Arc<STTx>,
    preflight_result:
        PreflightResult<AppTxQTransaction, TxConsequences, AppTxQJournalTag, AppTxQParentBatchId>,
    preclaim_result: PreclaimResult<AppTxQTransaction, AppTxQJournalTag, AppTxQParentBatchId>,
}

impl<'a> AppOpenLedgerTxQApplyRuntime<'a> {
    fn new(
        view: &'a mut AppOpenLedgerView,
        submit_view: &'a mut Sandbox<Ledger>,
        tx: Arc<STTx>,
        rules: Rules,
        flags: ApplyFlags,
        current_ledger_seq: u32,
        preclaim_ter: Ter,
    ) -> Self {
        let fee_field = get_field_by_symbol("sfFee");
        let fee_drops = if tx.is_field_present(fee_field) {
            tx.get_field_amount(fee_field).xrp().drops().max(0) as u64
        } else {
            0
        };
        let preflight_ter = match tx.check_sign(&rules) {
            Ok(()) => Ter::TES_SUCCESS,
            Err(_) => Ter::TEM_BAD_SIGNATURE,
        };
        let consequences = if is_tes_success(preflight_ter) {
            TxConsequences::new(fee_drops, tx.get_seq_proxy())
        } else {
            TxConsequences::from_preflight_result(preflight_ter)
        };
        let journal = "app_txq_submit".to_owned();
        let preflight_result = PreflightResult::new(
            Arc::clone(&tx),
            None,
            rules,
            consequences,
            flags,
            journal.clone(),
            preflight_ter,
        );
        let preclaim_result = PreclaimResult::new(
            current_ledger_seq,
            Arc::clone(&tx),
            None,
            flags,
            journal,
            preclaim_ter,
        );

        Self {
            view,
            submit_view,
            tx,
            preflight_result,
            preclaim_result,
        }
    }
}

impl QueueApplyExecutionRuntime<AppTxQTransaction, AppTxQJournalTag, AppTxQParentBatchId>
    for AppOpenLedgerTxQApplyRuntime<'_>
{
    fn run_preflight(
        &mut self,
    ) -> PreflightResult<AppTxQTransaction, TxConsequences, AppTxQJournalTag, AppTxQParentBatchId>
    {
        self.preflight_result.clone()
    }

    fn trace(&mut self, _message: &str) {}

    fn direct_apply(&mut self) -> ApplyResult {
        let txn_type = self.tx.get_txn_type();
        let ter = apply_submit_transactor_shell(self.submit_view, self.tx.as_ref(), txn_type);
        let applied = is_tes_success(ter) || is_tec_claim(ter);
        if applied {
            self.view.push_transaction(Arc::clone(&self.tx));
        }

        ApplyResult::new(ter, applied, false)
    }

    fn prepare_multitxn(&mut self, _adjustment: tx::QueueApplyViewAdjustment) -> bool {
        false
    }

    fn run_preclaim(
        &mut self,
        _view_source: tx::QueueApplyPreclaimViewSource,
    ) -> PreclaimResult<AppTxQTransaction, AppTxQJournalTag, AppTxQParentBatchId> {
        self.preclaim_result.clone()
    }

    fn run_try_clear(&mut self) -> ApplyResult {
        ApplyResult::new(Ter::TER_QUEUED, false, false)
    }

    fn apply_sandbox(&mut self) {}
}

#[derive(Clone)]
struct StandaloneAcceptedTx {
    transaction_id: Uint256,
    txn: Arc<Serializer>,
    metadata: Arc<Serializer>,
}

#[derive(Clone)]
struct StandaloneLedgerBuildView {
    inner: OpenView<Ledger>,
}

impl StandaloneLedgerBuildView {
    fn from_base(base: Arc<Ledger>, entries: &[StandaloneAcceptedTx]) -> Self {
        let mut inner = OpenView::new_closed(base);
        for entry in entries {
            inner
                .raw_tx_insert(
                    entry.transaction_id,
                    Arc::clone(&entry.txn),
                    Some(Arc::clone(&entry.metadata)),
                )
                .expect("standalone accepted tx overlay should insert");
        }
        Self { inner }
    }
}

impl crate::BuildLedgerView for StandaloneLedgerBuildView {
    fn open(&self) -> bool {
        self.inner.open()
    }

    fn tx_count(&self) -> usize {
        self.inner.tx_count()
    }

    fn apply_to_ledger(self, ledger: &mut Ledger) {
        self.inner
            .apply(ledger)
            .expect("standalone accepted tx overlay should apply");
    }
}

impl AcceptLedgerPendingRuntime {
    fn is_system_transaction(txn_type: TxType) -> bool {
        tx::run_with_system_transactor_txn_type_key(txn_type, |_| ()).is_ok()
    }

    fn read_sttx(tx: &AcceptLedgerPendingTransaction) -> Arc<STTx> {
        tx.transaction
            .lock()
            .expect("transaction mutex must not be poisoned")
            .get_s_transaction()
            .clone()
    }
}

fn queue_apply_preclaim_ter(view: &impl ReadView, tx: &STTx, current_ledger_seq: u32) -> Ter {
    if AcceptLedgerPendingRuntime::is_system_transaction(tx.get_txn_type()) {
        return Ter::TES_SUCCESS;
    }

    let preclaim_tx = QueueApplyPreclaimTx { tx };

    let seq_check = tx::run_transactor_check_seq_proxy(
        &preclaim_tx,
        |account| {
            view.read(account_keylet(
                Uint160::from_slice(account.data()).expect("account width should match Uint160"),
            ))
            .ok()
            .flatten()
        },
        |account_root| account_root.get_field_u32(get_field_by_symbol("sfSequence")),
        |account, tx_seq_proxy| {
            view.exists(protocol::ticket_keylet_from_seq_proxy(
                Uint160::from_slice(account.data()).expect("account width should match Uint160"),
                tx_seq_proxy,
            ))
            .unwrap_or(false)
        },
    );
    if !is_tes_success(seq_check) {
        return seq_check;
    }

    tx::run_transactor_check_prior_tx_and_last_ledger(
        current_ledger_seq,
        &preclaim_tx,
        |account| {
            view.read(account_keylet(
                Uint160::from_slice(account.data()).expect("account width should match Uint160"),
            ))
            .ok()
            .flatten()
        },
        |account_root| {
            if account_root.is_field_present(get_field_by_symbol("sfAccountTxnID")) {
                account_root.get_field_h256(get_field_by_symbol("sfAccountTxnID"))
            } else {
                Uint256::zero()
            }
        },
        |tx_id| view.tx_exists(*tx_id).unwrap_or(false),
    )
}

fn submit_confine_owner_count(current: u32, adjustment: i32) -> u32 {
    let result = current as i64 + adjustment as i64;
    if result < 0 {
        0
    } else if result > u32::MAX as i64 {
        u32::MAX
    } else {
        result as u32
    }
}

fn delete_submit_ticket<V: ledger::ApplyView>(
    view: &mut V,
    account: AccountID,
    account_state: &mut STLedgerEntry,
    tx_seq_proxy: SeqProxy,
) -> Ter {
    let account_uint160 =
        Uint160::from_slice(account.data()).expect("account width should match Uint160");
    let ticket_keylet = protocol::ticket_keylet_from_seq_proxy(account_uint160, tx_seq_proxy);
    let ticket = match view.peek(ticket_keylet) {
        Ok(Some(sle)) => Some(SubmitConsumedTicket {
            owner_page: sle.get_field_u64(get_field_by_symbol("sfOwnerNode")),
            sle,
        }),
        Ok(None) => None,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };

    let Some(ticket) = ticket else {
        return Ter::TEF_BAD_LEDGER;
    };

    if !ledger::dir_remove(
        view,
        &protocol::owner_dir_keylet(account_uint160),
        ticket.owner_page,
        *ticket.sle.key(),
        true,
    )
    .unwrap_or(false)
    {
        return Ter::TEF_BAD_LEDGER;
    }

    if !account_state.is_field_present(get_field_by_symbol("sfTicketCount")) {
        return Ter::TEF_BAD_LEDGER;
    }

    let ticket_count = account_state.get_field_u32(get_field_by_symbol("sfTicketCount"));
    if ticket_count == 1 {
        account_state.make_field_absent(get_field_by_symbol("sfTicketCount"));
    } else {
        account_state.set_field_u32(get_field_by_symbol("sfTicketCount"), ticket_count - 1);
    }

    let owner_count = account_state.get_field_u32(get_field_by_symbol("sfOwnerCount"));
    account_state.set_field_u32(
        get_field_by_symbol("sfOwnerCount"),
        submit_confine_owner_count(owner_count, -1),
    );

    let _ = view.erase(ticket.sle);
    Ter::TES_SUCCESS
}

pub fn apply_submit_transactor_shell<V: ledger::ApplyView>(
    view: &mut V,
    tx: &STTx,
    txn_type: TxType,
) -> Ter {
    let rules = view.rules();
    tx::with_transaction_apply_runtime(&rules, || {
        apply_submit_transactor_shell_impl(view, tx, txn_type, true)
    })
}

fn apply_submit_transactor_shell_impl<V: ledger::ApplyView>(
    view: &mut V,
    tx: &STTx,
    txn_type: TxType,
    run_batch_followup: bool,
) -> Ter {
    let account_field = get_field_by_symbol("sfAccount");

    // --- reference preflight1 checks ---
    // Bad account ID
    if tx.is_field_present(account_field) {
        let account = tx.get_account_id(account_field);
        if account.data().iter().all(|&b| b == 0) {
            return Ter::TEM_BAD_SRC_ACCOUNT;
        }
    }
    // Bad fee (must be native, non-negative)
    let fee_field = get_field_by_symbol("sfFee");
    if tx.is_field_present(fee_field) {
        let fee = tx.get_field_amount(fee_field);
        if !fee.native() || fee.negative() {
            return Ter::TEM_BAD_FEE;
        }
    }
    // Ticket + AccountTxnID is invalid
    let account_txn_id_field = get_field_by_symbol("sfAccountTxnID");
    if tx.get_seq_proxy().is_ticket() && tx.is_field_present(account_txn_id_field) {
        return Ter::TEM_INVALID;
    }

    let tx_object: &protocol::STObject = tx;
    if view
        .rules()
        .enabled(&protocol::feature_id("fixCleanup3_2_0"))
        && protocol::has_invalid_amount(tx_object)
    {
        return Ter::TEM_BAD_AMOUNT;
    }

    if !tx.is_field_present(account_field) {
        return handle_real_dispatch(view, tx, txn_type, None);
    }

    // Pseudo-transactions (Fee, Amendment, UNL_Modify) skip fee/sequence
    let is_pseudo = matches!(
        txn_type,
        TxType::FEE | TxType::AMENDMENT | TxType::UNL_MODIFY
    );
    if is_pseudo {
        return handle_real_dispatch(view, tx, txn_type, None);
    }

    let sequence_field = get_field_by_symbol("sfSequence");
    let balance_field = get_field_by_symbol("sfBalance");
    let account = tx.get_account_id(account_field);
    let fee_payer = tx.get_fee_payer();
    let account_uint160 =
        Uint160::from_slice(account.data()).expect("account width should match Uint160");
    let account_key = account_keylet(account_uint160);

    let Some(account_root) = view.peek(account_key).ok().flatten() else {
        return Ter::TER_NO_ACCOUNT;
    };

    let mut updated =
        STLedgerEntry::from_stobject(account_root.clone_as_object(), *account_root.key());
    let pre_fee_balance_drops = if updated.is_field_present(balance_field) {
        Some(updated.get_field_amount(balance_field).xrp().drops())
    } else {
        None
    };

    let consume_result = if tx.get_seq_proxy().is_seq() {
        updated.set_field_u32(sequence_field, tx.get_seq_proxy().value() + 1);
        Ter::TES_SUCCESS
    } else {
        delete_submit_ticket(view, account, &mut updated, tx.get_seq_proxy())
    };
    if !is_tes_success(consume_result) {
        return consume_result;
    }

    if fee_payer == account {
        if updated.is_field_present(balance_field) && tx.is_field_present(fee_field) {
            let balance_drops = updated.get_field_amount(balance_field).xrp().drops();
            let fee_drops = tx.get_field_amount(fee_field).xrp().drops();
            if balance_drops < fee_drops {
                // return tecINSUFF_FEE (claimed) — cap fee to actual balance, burn it,
                // consume sequence, discard all other state changes.
                // In an open ledger, return terINSUF_FEE_B (retry, no fee burned).
                // The build path is always a closed ledger.
                if balance_drops > 0 && !view.open() {
                    let actual_fee = balance_drops;
                    updated.set_field_amount(
                        balance_field,
                        STAmount::from_xrp_amount(XRPAmount::from_drops(0)),
                    );
                    let _ = view.update(Arc::new(updated));
                    let _ = view.destroy_xrp(XRPAmount::from_drops(actual_fee));
                    return Ter::TEC_INSUFF_FEE;
                }
                return Ter::TER_INSUF_FEE_B;
            }
            updated.set_field_amount(
                balance_field,
                STAmount::from_xrp_amount(XRPAmount::from_drops(balance_drops - fee_drops)),
            );
        }
    } else {
        let fee_payer_uint160 =
            Uint160::from_slice(fee_payer.data()).expect("fee payer width should match Uint160");
        let Some(fee_payer_sle) = view.peek(account_keylet(fee_payer_uint160)).ok().flatten()
        else {
            return Ter::TEF_INTERNAL;
        };
        let mut updated_fee_payer =
            STLedgerEntry::from_stobject(fee_payer_sle.clone_as_object(), *fee_payer_sle.key());
        let new_balance = updated_fee_payer
            .get_field_amount(balance_field)
            .xrp()
            .drops()
            - tx.get_field_amount(fee_field).xrp().drops();
        updated_fee_payer.set_field_amount(
            balance_field,
            STAmount::from_xrp_amount(XRPAmount::from_drops(new_balance)),
        );
        let _ = view.update(Arc::new(updated_fee_payer));
    }

    if updated.is_field_present(account_txn_id_field) {
        updated.set_field_h256(account_txn_id_field, tx.get_transaction_id());
    }

    let _ = view.update(Arc::new(updated));
    if tx.is_field_present(fee_field) {
        let fee_drops = tx.get_field_amount(fee_field).xrp().drops();
        if fee_drops > 0 {
            let _ = view.destroy_xrp(XRPAmount::from_drops(fee_drops));
        }
    }
    let mut ter = {
        // that failed to fully cross), discard all transactor state changes but keep the
        // fee deduction and sequence consumption that were already applied above.
        // We achieve this by running handle_real_dispatch in a nested FlowSandbox and
        // only applying it to the outer view when the result is NOT tecKILLED.
        let mut inner = ledger::FlowSandbox::new(view);
        let mut result = handle_real_dispatch(&mut inner, tx, txn_type, pre_fee_balance_drops);

        if protocol::is_tes_success(result) || protocol::is_tec_claim(result) {
            let fee_amt = if tx.is_field_present(fee_field) {
                tx.get_field_amount(fee_field).xrp()
            } else {
                protocol::XRPAmount::from_drops(0)
            };
            result = crate::state::invariants::check_invariants_for_tx(&inner, tx, result, fee_amt);
        }

        // tecOVERSIZE check
        if inner.item_count() > 32768 {
            result = Ter::TEC_OVERSIZE;
        }

        let do_offers = result == Ter::TEC_OVERSIZE || result == Ter::TEC_KILLED;
        let do_lines_or_mpts = result == Ter::TEC_INCOMPLETE;
        let do_nf_token_offers = result == Ter::TEC_EXPIRED;
        let do_credentials = result == Ter::TEC_EXPIRED;

        if !do_offers && !do_lines_or_mpts && !do_nf_token_offers && !do_credentials {
            if result != Ter::TEC_KILLED {
                // Apply inner sandbox changes to outer view (normal path)
                let _ = inner.apply();
            }
        } else {
            // Cleanup path
            let mut removed_offers = Vec::new();
            let mut removed_trust_lines = Vec::new();
            let mut removed_mpts = Vec::new();
            let mut expired_nft_offers = Vec::new();
            let mut expired_credentials = Vec::new();

            let erased_entries: Vec<(basics::base_uint::Uint256, Arc<protocol::STLedgerEntry>)> =
                inner
                    .items()
                    .iter()
                    .filter(|(_, entry)| entry.action == ledger::flow_sandbox::Action::Erase)
                    .map(|(k, entry)| (*k, entry.sle.clone()))
                    .collect();

            drop(inner); // discard transactor state changes

            for (index, after) in erased_entries {
                if let Ok(Some(before)) = view.peek(protocol::Keylet::new(after.get_type(), index))
                {
                    if do_offers && before.get_type() == protocol::LedgerEntryType::Offer {
                        let taker_pays = protocol::get_field_by_symbol("sfTakerPays");
                        if before.get_field_amount(taker_pays) == after.get_field_amount(taker_pays)
                        {
                            removed_offers.push(index);
                        }
                    }
                    if do_lines_or_mpts {
                        if before.get_type() == protocol::LedgerEntryType::RippleState {
                            removed_trust_lines.push(index);
                        } else if before.get_type() == protocol::LedgerEntryType::MPToken {
                            removed_mpts.push(index);
                        }
                    }
                    if do_nf_token_offers
                        && before.get_type() == protocol::LedgerEntryType::NFTokenOffer
                    {
                        expired_nft_offers.push(index);
                    }
                    if do_credentials && before.get_type() == protocol::LedgerEntryType::Credential
                    {
                        expired_credentials.push(index);
                    }
                }
            }

            if do_offers && !removed_offers.is_empty() {
                let mut count = 0;
                for index in removed_offers {
                    if let Ok(Some(sle)) = view.peek(protocol::Keylet::new(
                        protocol::LedgerEntryType::Offer,
                        index,
                    )) {
                        let account =
                            sle.get_account_id(protocol::get_field_by_symbol("sfAccount"));
                        let _ = crate::state::offer_create::offer_delete_pub(view, &account, sle);
                        count += 1;
                        if count == 1000 {
                            break;
                        }
                    }
                }
            }

            if result == Ter::TEC_EXPIRED && !expired_nft_offers.is_empty() {
                let mut count = 0;
                for index in expired_nft_offers {
                    if let Ok(Some(offer)) = view.peek(protocol::keylet::nft_offer_keylet(index)) {
                        let owner = offer.get_account_id(protocol::get_field_by_symbol("sfOwner"));
                        let owner_node =
                            offer.get_field_u64(protocol::get_field_by_symbol("sfOwnerNode"));
                        let owner_dir =
                            protocol::owner_dir_keylet(Uint160::from_void(owner.data()));
                        let _ = ledger::dir_remove(view, &owner_dir, owner_node, index, false);

                        let nftoken_id =
                            offer.get_field_h256(protocol::get_field_by_symbol("sfNFTokenID"));
                        let flags = offer.get_field_u32(protocol::get_field_by_symbol("sfFlags"));
                        let is_sell = (flags & protocol::lsfSellNFToken) != 0;
                        let nft_dir = if is_sell {
                            protocol::nft_sell_offers_keylet(nftoken_id)
                        } else {
                            protocol::nft_buy_offers_keylet(nftoken_id)
                        };
                        let nft_node = offer
                            .get_field_u64(protocol::get_field_by_symbol("sfNFTokenOfferNode"));
                        let _ = ledger::dir_remove(view, &nft_dir, nft_node, index, false);

                        if let Ok(Some(acct)) =
                            view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
                        {
                            let _ = ledger::adjust_owner_count(view, &acct, -1);
                        }
                        let _ = view.erase(offer);
                        count += 1;
                        if count == 1000 {
                            break;
                        }
                    }
                }
            }

            if result == Ter::TEC_INCOMPLETE {
                if !removed_trust_lines.is_empty() && removed_trust_lines.len() <= 500 {
                    for index in removed_trust_lines {
                        if let Ok(Some(sle)) = view.peek(protocol::Keylet::new(
                            protocol::LedgerEntryType::RippleState,
                            index,
                        )) {
                            let low = sle
                                .get_field_amount(protocol::get_field_by_symbol("sfLowLimit"))
                                .issue()
                                .account;
                            let high = sle
                                .get_field_amount(protocol::get_field_by_symbol("sfHighLimit"))
                                .issue()
                                .account;
                            let _ = crate::state::trust_set::trust_delete(view, &sle, &low, &high);
                        }
                    }
                }
                if !removed_mpts.is_empty() && removed_mpts.len() <= 2 {
                    for index in removed_mpts {
                        if let Ok(Some(sle)) = view.peek(protocol::Keylet::new(
                            protocol::LedgerEntryType::MPToken,
                            index,
                        )) {
                            let account =
                                sle.get_account_id(protocol::get_field_by_symbol("sfAccount"));
                            let node =
                                sle.get_field_u64(protocol::get_field_by_symbol("sfOwnerNode"));
                            let dir =
                                protocol::owner_dir_keylet(Uint160::from_void(account.data()));
                            let _ = ledger::dir_remove(view, &dir, node, index, false);
                            if let Ok(Some(acct)) = view
                                .peek(protocol::account_keylet(Uint160::from_void(account.data())))
                            {
                                let _ = ledger::adjust_owner_count(view, &acct, -1);
                            }
                            let _ = view.erase(sle);
                        }
                    }
                }
            }

            if result == Ter::TEC_EXPIRED && !expired_credentials.is_empty() {
                for index in expired_credentials {
                    if let Ok(Some(sle)) = view.peek(protocol::Keylet::new(
                        protocol::LedgerEntryType::Credential,
                        index,
                    )) {
                        match ledger::credential_helpers::delete_sle(view, sle) {
                            Ok(ter) if protocol::is_tes_success(ter) => {}
                            Ok(ter) => {
                                result = ter;
                                break;
                            }
                            Err(_) => {
                                result = Ter::TEF_BAD_LEDGER;
                                break;
                            }
                        }
                    }
                }
            }
        }
        result
    };

    if run_batch_followup && is_tes_success(ter) && txn_type == TxType::BATCH {
        ter = apply_submit_batch_followup(view, tx);
    }

    ter
}

fn apply_submit_batch_followup<V: ledger::ApplyView>(view: &mut V, batch_tx: &STTx) -> Ter {
    let batch_mode = BatchTransactionFlags::from_bits(batch_tx.get_flags());
    let mut whole_batch = ledger::FlowSandbox::new(view);
    let mut applied = 0_usize;

    for raw_tx in batch_tx
        .get_field_array(get_field_by_symbol("sfRawTransactions"))
        .iter()
        .cloned()
    {
        let inner_tx = STTx::from_stobject(raw_tx);
        let whole_batch_view: &mut dyn ledger::ApplyView = &mut whole_batch;
        let mut per_tx_batch_view = ledger::FlowSandbox::new(whole_batch_view);
        let result = apply_submit_transactor_shell_impl(
            &mut per_tx_batch_view,
            &inner_tx,
            inner_tx.get_txn_type(),
            false,
        );
        let inner_applied = is_tes_success(result) || is_tec_claim(result);

        if inner_applied {
            if per_tx_batch_view.apply().is_err() {
                return Ter::TEF_INTERNAL;
            }
            applied += 1;
        }

        if !is_tes_success(result) {
            if batch_mode.contains(BatchTransactionFlags::ALL_OR_NOTHING) {
                return Ter::TES_SUCCESS;
            }

            if batch_mode.contains(BatchTransactionFlags::UNTIL_FAILURE) {
                break;
            }
        } else if batch_mode.contains(BatchTransactionFlags::ONLY_ONE) {
            break;
        }
    }

    if applied != 0 && whole_batch.apply().is_err() {
        return Ter::TEF_INTERNAL;
    }

    Ter::TES_SUCCESS
}

impl
    AcceptLedgerPendingApplyRuntime<
        AppPlaceholder,
        Option<Arc<Ledger>>,
        Option<Arc<Ledger>>,
        AcceptLedgerPendingTransaction,
        Arc<crate::state::app_registry::AppJournal>,
        AppPlaceholder,
    > for AcceptLedgerPendingRuntime
{
    type Fee = u64;
    type PreflightError = ();
    type PreclaimError = ();
    type ApplyError = ();

    fn dispatch_preflight(
        &mut self,
        ctx: &tx::PreflightContext<
            AppPlaceholder,
            AcceptLedgerPendingTransaction,
            Arc<crate::state::app_registry::AppJournal>,
            AppPlaceholder,
        >,
        _txn_type: TxType,
    ) -> Result<(NotTec, TxConsequences), Self::PreflightError> {
        let sttx = Self::read_sttx(&ctx.tx);
        let fee_field = get_field_by_symbol("sfFee");
        let sequence_field = get_field_by_symbol("sfSequence");
        let fee_drops = if sttx.is_field_present(fee_field) {
            sttx.get_field_amount(fee_field).xrp().drops().max(0) as u64
        } else {
            0
        };
        let consequences = TxConsequences::new(
            fee_drops,
            SeqProxy::sequence(if sttx.is_field_present(sequence_field) {
                sttx.get_field_u32(sequence_field)
            } else {
                0
            }),
        );
        let result = match sttx.check_sign(&ctx.rules) {
            Ok(()) => Ter::TES_SUCCESS,
            Err(_) => Ter::TEM_BAD_SIGNATURE,
        };

        Ok((
            result,
            if is_tes_success(result) {
                consequences
            } else {
                TxConsequences::from_preflight_result(result)
            },
        ))
    }

    fn fallback_consequences(
        &mut self,
        ctx: &tx::PreflightContext<
            AppPlaceholder,
            AcceptLedgerPendingTransaction,
            Arc<crate::state::app_registry::AppJournal>,
            AppPlaceholder,
        >,
    ) -> TxConsequences {
        let sttx = Self::read_sttx(&ctx.tx);
        let fee_field = get_field_by_symbol("sfFee");
        let sequence_field = get_field_by_symbol("sfSequence");
        TxConsequences::new(
            if sttx.is_field_present(fee_field) {
                sttx.get_field_amount(fee_field).xrp().drops().max(0) as u64
            } else {
                0
            },
            SeqProxy::sequence(if sttx.is_field_present(sequence_field) {
                sttx.get_field_u32(sequence_field)
            } else {
                0
            }),
        )
    }

    fn dispatch_preclaim(
        &mut self,
        ctx: &tx::PreclaimContext<
            AppPlaceholder,
            Option<Arc<Ledger>>,
            AcceptLedgerPendingTransaction,
            Arc<crate::state::app_registry::AppJournal>,
            AppPlaceholder,
        >,
        txn_type: TxType,
    ) -> Result<Ter, Self::PreclaimError> {
        if Self::is_system_transaction(txn_type) {
            return Ok(Ter::TES_SUCCESS);
        }

        let Some(view) = ctx.view.as_ref() else {
            return Ok(Ter::TER_NO_ACCOUNT);
        };

        let sttx = Self::read_sttx(&ctx.tx);
        let account_field = get_field_by_symbol("sfAccount");
        let sequence_field = get_field_by_symbol("sfSequence");
        let account_id = sttx.get_account_id(account_field);
        let Some(account_root) = view
            .read(account_keylet(
                Uint160::from_slice(account_id.data()).expect("account width"),
            ))
            .ok()
            .flatten()
        else {
            return Ok(Ter::TER_NO_ACCOUNT);
        };

        if !sttx.is_field_present(sequence_field) {
            return Ok(Ter::TES_SUCCESS);
        }

        let tx_sequence = sttx.get_field_u32(sequence_field);
        let account_sequence = account_root.get_field_u32(sequence_field);

        Ok(if tx_sequence < account_sequence {
            Ter::TEF_PAST_SEQ
        } else if tx_sequence > account_sequence {
            Ter::TER_PRE_SEQ
        } else {
            Ter::TES_SUCCESS
        })
    }

    fn calculate_base_fee(
        &mut self,
        base: &Option<Arc<Ledger>>,
        tx: &AcceptLedgerPendingTransaction,
        txn_type: TxType,
    ) -> Self::Fee {
        let Some(ledger) = base.as_ref() else {
            return 0;
        };
        let normal_cost = ledger.fees().base;
        if txn_type == TxType::LOAN_PAY {
            let sttx = Self::read_sttx(tx);
            return crate::state::lending::calculate_loan_pay_base_fee(
                ledger.as_ref(),
                sttx.as_ref(),
                normal_cost,
            );
        }
        normal_cost
    }

    fn zero_fee(&mut self) -> Self::Fee {
        0
    }

    fn dispatch_apply(
        &mut self,
        ctx: &mut tx::ApplyContext<
            AppPlaceholder,
            Option<Arc<Ledger>>,
            Option<Arc<Ledger>>,
            AcceptLedgerPendingTransaction,
            Self::Fee,
            Arc<crate::state::app_registry::AppJournal>,
            AppPlaceholder,
        >,
        txn_type: TxType,
    ) -> Result<ApplyResult, Self::ApplyError> {
        if Self::is_system_transaction(txn_type) {
            return Ok(ApplyResult::new(Ter::TES_SUCCESS, true, false));
        }

        let result = ctx.preclaim_result;
        Ok(ApplyResult::new(
            result,
            is_tes_success(result) || is_tec_claim(result),
            false,
        ))
    }
}

impl LedgerAcceptor for ConsensusLedgerAcceptor {
    fn accept_ledger(
        &self,
        closed_seq: u32,
        close_time: u32,
        base_fee_drops: u64,
    ) -> Result<u32, String> {
        let root = self.root.clone();
        let job_queue = Arc::clone(&self.job_queue);
        let name = format!("AcceptLedger#{closed_seq}");

        if !job_queue.add_job(crate::job::job_types::JobType::Accept, name, move || {
            let _ = root.accept_ledger(closed_seq, close_time, base_fee_drops);
        }) {
            return Err("accept job queue is stopping".to_owned());
        }

        let dispatch_queue = Arc::clone(&self.job_queue);
        std::mem::drop(self.basic_app.spawn(async move {
            let _ = tokio::task::spawn_blocking(move || {
                let _ = dispatch_queue.dispatch_next_job();
            })
            .await;
        }));

        Ok(closed_seq.saturating_add(1))
    }

    fn consensus_built(&self, ledger: Arc<ledger::Ledger>) -> Result<(), String> {
        let root = self.root.clone();
        let job_queue = Arc::clone(&self.job_queue);
        let seq = ledger.header().seq;
        let name = format!("ConsensusBuilt#{seq}");

        if !job_queue.add_job(crate::job::job_types::JobType::Accept, name, move || {
            root.on_consensus_built_ledger(Arc::clone(&ledger));
        }) {
            return Err("consensus_built job queue is stopping".to_owned());
        }

        let dispatch_queue = Arc::clone(&self.job_queue);
        std::mem::drop(self.basic_app.spawn(async move {
            let _ = tokio::task::spawn_blocking(move || {
                let _ = dispatch_queue.dispatch_next_job();
            })
            .await;
        }));

        Ok(())
    }

    fn consensus_closed_ledger(&self) -> Option<Arc<Ledger>> {
        self.root.consensus_closed_ledger()
    }

    fn consensus_previous_ledger(&self) -> Option<Arc<Ledger>> {
        self.root.consensus_previous_ledger()
    }

    fn node_fetcher(
        &self,
    ) -> Option<
        Arc<
            dyn Fn(
                    basics::sha_map_hash::SHAMapHash,
                ) -> Option<
                    basics::memory::intrusive_pointer::SharedIntrusive<
                        shamap::nodes::tree_node::SHAMapTreeNode,
                    >,
                > + Send
                + Sync,
        >,
    > {
        let guard = self.shared_node_store.read().ok()?;
        let ns = guard.as_ref();
        if ns.is_none() {
            static FETCH_MISS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if FETCH_MISS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 3 {
                tracing::debug!(target: "consensus",
                    "[consensus_fetcher] shared_node_store is None — node store not yet attached"
                );
            }
            return None;
        }
        let ns = ns?.clone();
        Some(Arc::new(move |hash| {
            let data = match &ns {
                crate::shamap::shamap_store_backend::SHAMapStoreNodeStore::Single(db) => db
                    .fetch_node_object(
                        hash.as_uint256(),
                        0,
                        nodestore::FetchType::Synchronous,
                        false,
                    ),
                crate::shamap::shamap_store_backend::SHAMapStoreNodeStore::Rotating(db) => db
                    .fetch_node_object(
                        hash.as_uint256(),
                        0,
                        nodestore::FetchType::Synchronous,
                        false,
                    ),
            }?;
            shamap::nodes::tree_node::SHAMapTreeNode::make_from_prefix(data.data(), hash).ok()
        }))
    }
}

impl ApplicationRoot {
    pub fn clone_ledger_acceptor(&self) -> Arc<dyn LedgerAcceptor> {
        Arc::new(ConsensusLedgerAcceptor {
            root: self.clone(),
            job_queue: Arc::clone(&self.job_queue),
            basic_app: Arc::clone(&self.basic_app),
            shared_node_store: Arc::clone(&self.shared_consensus_node_store),
        })
    }
    pub fn new(worker_threads: usize) -> std::io::Result<Self> {
        Self::with_options(ApplicationRootOptions {
            io_threads: worker_threads,
            job_queue_threads: worker_threads.max(1),
            ..ApplicationRootOptions::default()
        })
    }

    pub fn with_runtime_bindings(
        options: ApplicationRootOptions,
        runtime_bindings: RuntimeBindings,
    ) -> std::io::Result<Self> {
        let mut root = Self::with_options(options)?;
        root.set_runtime_bindings(runtime_bindings);
        Ok(root)
    }

    pub fn with_options(options: ApplicationRootOptions) -> std::io::Result<Self> {
        let ApplicationRootOptions {
            io_threads,
            job_queue_threads,
            start_valid,
            elb_support,
            standalone,
            start_type,
            start_ledger,
            import,
            quorum,
            collector_params,
            load_manager_timing,
        } = options;

        let mut registry = ApplicationRegistryOwners::new().map_err(std::io::Error::other)?;
        registry.config.standalone = standalone;
        registry.config.start_up = start_type;
        registry.config.start_ledger = start_ledger;
        registry.config.do_import = import;
        if let Some(q) = quorum {
            registry.config.validation_quorum = q;
        }
        let perf_log = Arc::clone(
            registry
                .perf_log
                .as_ref()
                .expect("application root must own a perf log"),
        );
        let job_queue = JobQueue::with_worker_threads(job_queue_threads.max(1));
        let load_fee_track = Arc::new(SharedLoadFeeTrack::default());
        let collector_manager = CollectorManager::new(collector_params);
        let load_manager = LoadManager::with_timing(
            job_queue.clone(),
            load_fee_track.clone(),
            Arc::new(AppLoadManagerEvents {
                collector_manager: collector_manager.clone(),
            }),
            registry.logs.journal("load_manager"),
            load_manager_timing,
        );
        let time_keeper = Arc::new(TimeKeeper::new());
        let close_time_provider = Arc::clone(&time_keeper)
            as Arc<dyn crate::ledger::ledger_master_state::LedgerMasterCloseTimeProvider>;
        let ledger_master_state = Arc::new(SharedLedgerMasterState::new(close_time_provider));
        let validations = SharedAppValidations::new(
            Arc::clone(&time_keeper),
            Arc::clone(&ledger_master_state),
            registry.logs.journal("Validations"),
        );
        let validators = Arc::new(ValidatorList::new(
            ManifestCache::new(),
            ManifestCache::new(),
            SystemValidatorListClock,
            std::env::temp_dir().join("xrpld-application-root-validator-list"),
            None,
        ));
        let _ = validators.load(None, &[], &[], None);

        let network_ops_state = Arc::new(SharedNetworkOpsState::new(if start_valid {
            NetworkOpsOperatingMode::Full
        } else {
            NetworkOpsOperatingMode::Disconnected
        }));

        *registry
            .network_ops_state_sink
            .lock()
            .expect("network_ops_state_sink mutex poisoned") = Some(Arc::clone(&network_ops_state));

        Ok(Self {
            basic_app: Arc::new(BasicApp::new(io_threads)?),
            job_queue: Arc::new(job_queue.clone()),
            time_keeper: Arc::clone(&time_keeper),
            stop_tree: Arc::new(StopTree::new("application")),
            collector_manager: Arc::new(collector_manager),
            load_manager: Arc::new(load_manager),
            load_fee_track,
            registry,
            node_store_scheduler: Arc::new(NodeStoreScheduler::new(job_queue)),
            node_family: None,
            resolver_runtime: None,
            overlay_runtime: None,
            overlay_status: None,
            server_ports_setup: None,
            published_server_ports: None,
            status_metrics: Some(Arc::clone(&perf_log) as Arc<dyn StatusMetricsSource>),
            network_ops_state,
            network_ops_runtime: None,
            network_ops_validation_runtime: None,
            ledger_master_runtime: None,
            consensus_runtime: None,
            ledger_master_state,
            transaction_master: Arc::new(TransactionMaster::new()),
            validations,
            validators,
            status_rpc_state: Arc::new(StatusRpcState::new()),
            amendment_status: Arc::new(AmendmentStatus::new()),
            elb_support,
            node_identity: None,
            validation_public_key: None,
            runtime_bindings: RuntimeBindings {
                grpc: GrpcRuntime::default(),
                ..RuntimeBindings::default()
            },
            shamap_store_service: None,
            shared_consensus_node_store: Arc::new(std::sync::RwLock::new(None)),
        })
    }

    pub fn basic_app(&self) -> &BasicApp {
        &self.basic_app
    }

    pub fn job_queue(&self) -> &JobQueue {
        &self.job_queue
    }

    pub fn collector_manager(&self) -> &CollectorManager {
        &self.collector_manager
    }

    pub fn load_manager(&self) -> &LoadManager {
        &self.load_manager
    }

    pub fn fd_required(&self) -> usize {
        let mut needed = 128usize;
        if let Some(setup) = self.server_ports_setup.as_ref() {
            needed += setup.fd_required();
        }
        if let Some(service) = self.shamap_store_service.as_ref() {
            needed += service.component().fd_required().max(5);
        }
        if let Some(node_store) = self.registry.node_store.as_ref() {
            needed += node_store.fd_required().max(5) as usize;
        }

        needed = needed.max(self.runtime_bindings().fd_required());
        needed.max(1024)
    }

    pub fn load_fee_track(&self) -> Arc<SharedLoadFeeTrack> {
        Arc::clone(&self.load_fee_track)
    }

    pub fn open_ledger(&self) -> &SharedAppOpenLedger {
        &self.registry.open_ledger
    }

    pub fn order_book_db(&self) -> Arc<OrderBookDB> {
        Arc::clone(&self.registry.order_book_db)
    }

    pub fn live_current_ledger_index(&self) -> Option<u32> {
        let current_index = self.registry.open_ledger.current().ledger_current_index;
        (current_index != 0).then_some(current_index)
    }

    pub fn tx_q(&self) -> &SharedAppTxQ {
        &self.registry.tx_q
    }

    pub fn tx_q_account_txs(
        &self,
        account_id: AccountID,
    ) -> Vec<TxDetails<AppTxQTransaction, AppTxQAccount>> {
        self.registry.tx_q.current_account_txs(account_id)
    }

    pub fn tx_q_metrics(&self) -> QueueTxQMetrics {
        let current = self.registry.open_ledger.current();
        let mut lock = AppTxQLock;
        self.registry.tx_q.get_metrics(&mut lock, current.as_ref())
    }

    pub fn tx_q_rpc_report(&self) -> QueueTxQRpcReport {
        let current = self.registry.open_ledger.current();
        let mut lock = AppTxQLock;
        self.registry
            .tx_q
            .get_rpc_fee_report(&mut lock, current.as_ref())
    }

    fn validated_fee_levels_for_closed_ledger(&self, ledger: &Ledger) -> Vec<u64> {
        let fee_field = get_field_by_symbol("sfFee");
        let calculated_base_fee_drops = i64::try_from(ledger.fees().base).unwrap_or(i64::MAX);

        ledger
            .tx_snapshot()
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|(tx, _meta)| {
                        let fee_paid_drops = if tx.is_field_present(fee_field) {
                            tx.get_field_amount(fee_field).xrp().drops()
                        } else {
                            0
                        };

                        evaluate_fee_level_paid(QueueFeeLevelPaidInputs {
                            calculated_base_fee_drops,
                            fee_paid_drops,
                            default_base_fee_drops: calculated_base_fee_drops,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn process_closed_ledger_txq(
        &self,
        ledger: &Ledger,
        time_leap: bool,
    ) -> tx::ClosedLedgerMaintenanceWithMetrics<AppTxQAccount> {
        let mut lock = AppTxQLock;
        self.registry.tx_q.process_closed_ledger(
            &mut lock,
            self,
            &AppClosedLedgerTxQView { ledger },
            time_leap,
        )
    }

    fn rebuild_open_ledger_after_close(
        &self,
        next_open_index: u32,
        base_fee_drops: u64,
        parent_hash: Uint256,
    ) {
        let mut retries = Vec::<AppOpenLedgerTxRecord>::new();
        self.open_ledger().accept(
            || AppOpenLedgerView::with_parent_hash(next_open_index, base_fee_drops, parent_hash),
            &|_: &Uint256| false,
            std::iter::empty::<AppOpenLedgerTxRecord>(),
            false,
            &mut retries,
            ApplyFlags::NONE,
            &mut |view: &mut AppOpenLedgerView, tx: &AppOpenLedgerTxRecord, _flags| {
                view.push_transaction(Arc::clone(&tx.tx));
                ApplyResult::new(Ter::TES_SUCCESS, true, false)
            },
            &mut |view: &mut AppOpenLedgerView, tx: &AppOpenLedgerTxRecord, _flags| {
                view.push_transaction(Arc::clone(&tx.tx));
            },
            Some(|view: &mut AppOpenLedgerView| {
                let snapshot = AppOpenLedgerTxQAcceptView {
                    open_ledger_tx_count: view.tx_ids().len(),
                    parent_hash: view.parent_hash,
                };
                let mut runtime = AppOpenLedgerTxQAcceptRuntime { view };
                let mut lock = AppTxQLock;
                self.registry
                    .tx_q
                    .accept(&mut lock, &mut runtime, &snapshot)
                    .ledger_changed
            }),
            &mut |_tx_id: &Uint256| false,
            &mut |_tx: &AppOpenLedgerTxRecord| {},
        );
    }

    fn rebuild_open_ledger_after_consensus(
        &self,
        next_open_index: u32,
        base_fee_drops: u64,
        parent_hash: Uint256,
    ) {
        let current_locals = self.open_ledger().current().ordered_txs();
        let mut retries = Vec::<AppOpenLedgerTxRecord>::new();
        self.open_ledger().accept(
            || AppOpenLedgerView::with_parent_hash(next_open_index, base_fee_drops, parent_hash),
            &|_: &Uint256| false,
            current_locals,
            false,
            &mut retries,
            ApplyFlags::NONE,
            &mut |view: &mut AppOpenLedgerView, tx: &AppOpenLedgerTxRecord, _flags| {
                view.push_transaction(Arc::clone(&tx.tx));
                ApplyResult::new(Ter::TES_SUCCESS, true, false)
            },
            &mut |view: &mut AppOpenLedgerView, tx: &AppOpenLedgerTxRecord, _flags| {
                view.push_transaction(Arc::clone(&tx.tx));
            },
            Some(|view: &mut AppOpenLedgerView| {
                let snapshot = AppOpenLedgerTxQAcceptView {
                    open_ledger_tx_count: view.tx_ids().len(),
                    parent_hash: view.parent_hash,
                };
                let mut runtime = AppOpenLedgerTxQAcceptRuntime { view };
                let mut lock = AppTxQLock;
                self.registry
                    .tx_q
                    .accept(&mut lock, &mut runtime, &snapshot)
                    .ledger_changed
            }),
            &mut |_tx_id: &Uint256| false,
            &mut |_tx: &AppOpenLedgerTxRecord| {},
        );
    }

    pub fn on_consensus_built_ledger(&self, ledger: Arc<Ledger>) {
        let ledger = self.ledger_with_node_fetcher(ledger);
        let _ = self.process_closed_ledger_txq(ledger.as_ref(), false);

        let next_open_index = ledger.header().seq.saturating_add(1);
        let next_open_parent_hash = *ledger.header().hash.as_uint256();
        self.rebuild_open_ledger_after_consensus(
            next_open_index,
            ledger.fees().base,
            next_open_parent_hash,
        );

        if let Some(runtime) = self.ledger_master_runtime() {
            // rounds can look up the parent via get_ledger_by_hash.
            runtime
                .ledger_master()
                .ledger_history()
                .insert(Arc::clone(&ledger), false);
            runtime
                .ledger_master()
                .set_closed_ledger(Arc::clone(&ledger));
        }
        self.on_closed_ledger(Arc::clone(&ledger));
        self.set_status_rpc_current_ledger_index(Some(next_open_index));
        self.set_status_rpc_queue_report(Some(self.tx_q_rpc_report()));
    }

    pub fn perf_log(&self) -> Arc<PerfLogImp> {
        Arc::clone(
            self.registry
                .perf_log
                .as_ref()
                .expect("application root must own a perf log"),
        )
    }

    pub fn attach_perf_log(&mut self, perf_log: Arc<PerfLogImp>) -> Option<Arc<PerfLogImp>> {
        let previous = self.registry.attach_perf_log(Arc::clone(&perf_log));
        self.status_metrics = Some(Arc::clone(&perf_log) as Arc<dyn StatusMetricsSource>);
        previous
    }

    pub fn wallet_db(&self) -> Arc<DatabaseCon> {
        Arc::clone(&self.registry.wallet_db)
    }

    pub fn inbound_ledgers(&self) -> &AppInboundLedgers {
        &self.registry.inbound_ledgers
    }

    pub fn inbound_transactions(&self) -> &AppInboundTransactions {
        &self.registry.inbound_transactions
    }

    pub fn server_handler(&self) -> Arc<AppServerHandler> {
        Arc::clone(&self.registry.server_handler)
    }

    pub fn accepted_ledger_cache(&self) -> &AppAcceptedLedgerCache {
        &self.registry.accepted_ledger_cache
    }

    pub fn peer_reservations(&self) -> &PeerReservationTable<PublicKey> {
        self.registry.peer_reservations.as_ref()
    }

    pub fn peer_reservation_source(&self) -> Arc<dyn PeerReservationSource> {
        Arc::clone(&self.registry.peer_reservations) as Arc<dyn PeerReservationSource>
    }

    pub fn shared_cluster(&self) -> Arc<Cluster> {
        Arc::clone(&self.registry.cluster)
    }

    pub fn wire_overlay_cluster(&self, overlay: &OverlayImpl) {
        overlay.set_cluster_source(self.shared_cluster());
    }

    pub fn wire_overlay_peer_reservations(&self, overlay: &OverlayImpl) {
        overlay.set_peer_reservation_source(self.peer_reservation_source());
    }

    pub fn wire_overlay_membership_sources(&self, overlay: &OverlayImpl) {
        self.wire_overlay_cluster(overlay);
        self.wire_overlay_peer_reservations(overlay);
    }

    pub fn load_peer_reservations(&self) -> Result<bool, String> {
        xrpl_core::load_peer_reservations_from_registry(self)
    }

    pub fn logs(&self) -> Arc<AppLogs> {
        Arc::clone(&self.registry.logs)
    }

    pub fn load_monitor_journal_factory(&self) -> Arc<dyn LoadMonitorJournalFactory> {
        self.registry.load_monitor_journal_factory.clone()
    }

    pub fn config(&self) -> &AppConfig {
        &self.registry.config
    }

    pub fn standalone(&self) -> bool {
        self.registry.config.standalone
    }

    pub fn network_id(&self) -> u32 {
        self.registry.network_id_service.get_network_id()
    }

    pub fn path_search_max(&self) -> u32 {
        self.registry.config.path_search_max
    }

    pub fn relay_untrusted_validations(&self) -> bool {
        self.registry.config.relay_untrusted_validations
    }

    pub fn path_search_old(&self) -> u32 {
        self.registry.config.path_search_old
    }

    pub fn path_search(&self) -> u32 {
        self.registry.config.path_search
    }

    pub fn path_search_fast(&self) -> u32 {
        self.registry.config.path_search_fast
    }

    pub fn set_path_search_levels(
        &mut self,
        path_search_old: u32,
        path_search: u32,
        path_search_fast: u32,
    ) {
        self.registry.config.path_search_old = path_search_old;
        self.registry.config.path_search = path_search;
        self.registry.config.path_search_fast = path_search_fast;
    }

    pub fn set_path_search_max(&mut self, path_search_max: u32) -> u32 {
        std::mem::replace(&mut self.registry.config.path_search_max, path_search_max)
    }

    pub fn set_relay_untrusted_validations(&mut self, relay_untrusted_validations: bool) -> bool {
        let previous = std::mem::replace(
            &mut self.registry.config.relay_untrusted_validations,
            relay_untrusted_validations,
        );
        if let Some(runtime) = self.network_ops_validation_runtime.as_ref() {
            let _ = runtime.set_relay_untrusted_validations(relay_untrusted_validations);
        }
        previous
    }

    pub fn node_store(&self) -> &Option<crate::shamap::shamap_store_backend::SHAMapStoreNodeStore> {
        &self.registry.node_store
    }

    /// Create a node_fetcher closure from the node store. Used by ConsensusLedgerAcceptor.
    pub fn node_fetcher_from_store(
        &self,
    ) -> Option<
        std::sync::Arc<
            dyn Fn(
                    basics::sha_map_hash::SHAMapHash,
                ) -> Option<
                    basics::memory::intrusive_pointer::SharedIntrusive<
                        shamap::nodes::tree_node::SHAMapTreeNode,
                    >,
                > + Send
                + Sync,
        >,
    > {
        let ns = self.node_store().as_ref()?.clone();
        let node_family = self.node_family();
        Some(std::sync::Arc::new(move |hash| {
            if let Some(family) = node_family.as_ref()
                && let Some(node) = family.fetch_cached_node(hash, 0)
            {
                full_sync_debug!(
                    "[full_debug][node_fetch] source=family_cache hash={} result=hit",
                    hash
                );
                return Some(node);
            }

            let data = match &ns {
                crate::shamap::shamap_store_backend::SHAMapStoreNodeStore::Single(db) => db
                    .fetch_node_object(
                        hash.as_uint256(),
                        0,
                        nodestore::FetchType::Synchronous,
                        false,
                    ),
                crate::shamap::shamap_store_backend::SHAMapStoreNodeStore::Rotating(db) => db
                    .fetch_node_object(
                        hash.as_uint256(),
                        0,
                        nodestore::FetchType::Synchronous,
                        false,
                    ),
            };
            let Some(data) = data else {
                full_sync_debug!(
                    "[full_debug][node_fetch] source=nodestore hash={} result=miss",
                    hash
                );
                return None;
            };
            let decoded =
                shamap::nodes::tree_node::SHAMapTreeNode::make_from_prefix(data.data(), hash).ok();
            full_sync_debug!(
                "[full_debug][node_fetch] source=nodestore hash={} result={} bytes={}",
                hash,
                if decoded.is_some() {
                    "hit"
                } else {
                    "decode_fail"
                },
                data.data().len()
            );
            decoded
        }))
    }

    pub fn node_writer_from_store(
        &self,
    ) -> Option<
        std::sync::Arc<
            dyn Fn(ledger::LedgerNodeObjectType, basics::base_uint::Uint256, Vec<u8>, u32)
                + Send
                + Sync,
        >,
    > {
        let ns = self.node_store().as_ref()?.clone();
        Some(std::sync::Arc::new(
            move |object_type, hash, data, ledger_seq| match &ns {
                crate::shamap::shamap_store_backend::SHAMapStoreNodeStore::Single(db) => {
                    db.store(to_nodestore_type(object_type), data, hash, ledger_seq);
                }
                crate::shamap::shamap_store_backend::SHAMapStoreNodeStore::Rotating(db) => {
                    db.store(to_nodestore_type(object_type), data, hash, ledger_seq);
                }
            },
        ))
    }

    pub fn attach_node_store(
        &mut self,
        node_store: Option<crate::shamap::shamap_store_backend::SHAMapStoreNodeStore>,
    ) -> Option<crate::shamap::shamap_store_backend::SHAMapStoreNodeStore> {
        // Also populate the shared consensus node store so ConsensusLedgerAcceptor
        // can access it (it was created before the node store was attached).
        if let Some(ref ns) = node_store {
            if let Ok(mut guard) = self.shared_consensus_node_store.write() {
                *guard = Some(ns.clone());
            }
        }
        std::mem::replace(&mut self.registry.node_store, node_store)
    }

    pub fn relational_database(
        &self,
    ) -> &Option<Arc<crate::shamap::shamap_store_relational::SqliteSHAMapStoreRelational>> {
        &self.registry.relational_database
    }

    pub fn attach_relational_database(
        &mut self,
        relational_database: Option<
            Arc<crate::shamap::shamap_store_relational::SqliteSHAMapStoreRelational>,
        >,
    ) -> Option<Arc<crate::shamap::shamap_store_relational::SqliteSHAMapStoreRelational>> {
        std::mem::replace(&mut self.registry.relational_database, relational_database)
    }

    pub fn attach_ledger_db(
        &mut self,
        ledger_db: Option<std::sync::Arc<rdb::LedgerDb>>,
    ) -> Option<std::sync::Arc<rdb::LedgerDb>> {
        std::mem::replace(&mut self.registry.ledger_db, ledger_db)
    }

    /// Return a reference to the ledger header database, if open.
    pub fn ledger_db(&self) -> Option<&std::sync::Arc<rdb::LedgerDb>> {
        self.registry.ledger_db.as_ref()
    }

    pub fn build_ledger_persistence_runtime(&self) -> crate::AppLedgerPersistenceRuntime {
        crate::AppLedgerPersistenceRuntime::new(
            self.registry.relational_database.clone(),
            self.registry.node_store.clone(),
            Arc::clone(&self.transaction_master),
            self.registry.network_id_service.get_network_id(),
            self.registry.ledger_db.clone(),
        )
    }

    pub fn node_store_scheduler(&self) -> &NodeStoreScheduler {
        &self.node_store_scheduler
    }

    pub fn time_keeper(&self) -> &TimeKeeper<SystemTimeKeeperClock> {
        self.time_keeper.as_ref()
    }

    pub fn shared_time_keeper(&self) -> Arc<TimeKeeper<SystemTimeKeeperClock>> {
        Arc::clone(&self.time_keeper)
    }

    pub fn stop_tree(&self) -> &StopTree {
        &self.stop_tree
    }

    pub fn node_family(&self) -> Option<Arc<dyn NodeFamilyRuntime>> {
        self.node_family.as_ref().map(Arc::clone)
    }

    pub fn resolver_runtime(&self) -> Option<Arc<AppResolverRuntime>> {
        self.resolver_runtime.as_ref().map(Arc::clone)
    }

    pub fn overlay_runtime(&self) -> Option<Arc<AppOverlayRuntime>> {
        self.overlay_runtime.as_ref().map(Arc::clone)
    }

    pub fn attach_node_family(
        &mut self,
        node_family: Arc<dyn NodeFamilyRuntime>,
    ) -> Option<Arc<dyn NodeFamilyRuntime>> {
        self.node_family.replace(node_family)
    }

    pub fn attach_resolver_runtime(
        &mut self,
        resolver_runtime: Arc<AppResolverRuntime>,
    ) -> Option<Arc<AppResolverRuntime>> {
        self.runtime_bindings.resolver = Some(resolver_runtime.clone());
        self.resolver_runtime.replace(resolver_runtime)
    }

    pub fn attach_default_resolver_runtime(&mut self) -> Arc<AppResolverRuntime> {
        if let Some(runtime) = self.resolver_runtime() {
            return runtime;
        }

        let runtime = Arc::new(AppResolverRuntime::default());
        let _ = self.attach_resolver_runtime(Arc::clone(&runtime));
        runtime
    }

    pub fn overlay_status(&self) -> Option<Arc<dyn OverlayStatusSource>> {
        self.overlay_status.as_ref().map(Arc::clone)
    }

    pub fn attach_overlay_status(
        &mut self,
        overlay_status: Arc<dyn OverlayStatusSource>,
    ) -> Option<Arc<dyn OverlayStatusSource>> {
        self.overlay_status.replace(overlay_status)
    }

    pub fn attach_overlay_runtime(
        &mut self,
        overlay_runtime: Arc<AppOverlayRuntime>,
    ) -> Option<Arc<AppOverlayRuntime>> {
        let overlay = overlay_runtime.overlay();
        self.wire_overlay_membership_sources(overlay.as_ref());
        let overlay_status: Arc<dyn OverlayStatusSource> = overlay;
        self.overlay_status = Some(overlay_status);
        self.runtime_bindings.overlay = Some(overlay_runtime.clone());
        self.overlay_runtime.replace(overlay_runtime)
    }

    pub fn attach_configured_overlay_runtime(
        &mut self,
        config: &basics::basic_config::BasicConfig,
        handoff: Arc<dyn OverlayHandoff>,
    ) -> Result<Arc<AppOverlayRuntime>, String> {
        let runtime = build_overlay_runtime(
            config,
            self.server_ports_setup.as_deref(),
            handoff,
            Some(self.network_ops_mode_owner()),
            Some(Arc::clone(&self.status_rpc_state)),
        )?;
        self.registry.network_id_service =
            FixedNetworkIdService::new(runtime.network_id().unwrap_or(0));
        if let Some(validation_runtime) = self.network_ops_validation_runtime.as_ref() {
            let _ = validation_runtime
                .set_network_id(self.registry.network_id_service.get_network_id());
        }
        let _ = self.attach_overlay_runtime(Arc::clone(&runtime));
        Ok(runtime)
    }

    pub fn load_cluster_nodes_from_config(
        &self,
        config: &basics::basic_config::BasicConfig,
    ) -> Result<bool, String> {
        let entries = config.section("cluster_nodes").values();
        if entries.is_empty() {
            return Ok(false);
        }
        if self.shared_cluster().load(entries) {
            Ok(true)
        } else {
            Err("Invalid entry in cluster configuration.".to_owned())
        }
    }

    pub fn attach_default_node_family(&mut self) -> Arc<dyn NodeFamilyRuntime> {
        if let Some(node_family) = self.node_family() {
            return node_family;
        }

        let profile =
            crate::NodeSizeResourceProfile::for_node_size(self.status_rpc_node_size().as_deref());
        let family: Arc<dyn NodeFamilyRuntime> = Arc::new(NodeFamily::new(SHAMapFamily::new(
            Arc::new(TreeNodeCache::new(
                "app-bootstrap-node-family",
                profile.tree_cache_size,
                Duration::seconds(profile.tree_cache_age_seconds),
                MonotonicClock::default(),
            )),
            NullFullBelowCache::new(0),
            NullNodeFetcher,
            NullMissingNodeReporter,
        )));
        let _ = self.attach_node_family(Arc::clone(&family));
        let _ = self.wire_node_family_reset();
        family
    }

    pub fn server_ports_setup(&self) -> Option<Arc<ServerPortsSetup>> {
        self.server_ports_setup.as_ref().map(Arc::clone)
    }

    pub fn attach_server_ports_setup(
        &mut self,
        server_ports_setup: Arc<ServerPortsSetup>,
    ) -> Option<Arc<ServerPortsSetup>> {
        self.server_ports_setup.replace(server_ports_setup)
    }

    pub fn attach_server_ports_from_config(
        &mut self,
        config: &basics::basic_config::BasicConfig,
        standalone: bool,
    ) -> Result<bool, String> {
        if self.server_ports_setup.is_some() {
            return Ok(false);
        }
        if !config.exists("server") {
            return Ok(false);
        }

        let setup = Arc::new(build_server_ports_setup(config, standalone)?);
        let _ = self.attach_server_ports_setup(setup);
        Ok(true)
    }

    pub fn published_server_ports(&self) -> Option<Arc<dyn PublishedServerPortsSource>> {
        if let Some(setup) = self.server_ports_setup.as_ref() {
            return Some(Arc::clone(setup) as Arc<dyn PublishedServerPortsSource>);
        }
        self.published_server_ports.as_ref().map(Arc::clone)
    }

    pub fn attach_published_server_ports(
        &mut self,
        published_server_ports: Arc<dyn PublishedServerPortsSource>,
    ) -> Option<Arc<dyn PublishedServerPortsSource>> {
        self.published_server_ports.replace(published_server_ports)
    }

    pub fn status_metrics(&self) -> Option<Arc<dyn StatusMetricsSource>> {
        if let Some(status_metrics) = self.status_metrics.as_ref() {
            return Some(Arc::clone(status_metrics));
        }

        self.registry
            .perf_log
            .as_ref()
            .map(|perf_log| Arc::clone(perf_log) as Arc<dyn StatusMetricsSource>)
    }

    pub fn attach_status_metrics(
        &mut self,
        status_metrics: Arc<dyn StatusMetricsSource>,
    ) -> Option<Arc<dyn StatusMetricsSource>> {
        self.status_metrics.replace(status_metrics)
    }

    pub fn node_identity(&self) -> Option<(PublicKey, SecretKey)> {
        self.node_identity
            .as_ref()
            .map(|(public, secret)| (*public, secret.clone()))
    }

    pub const fn validation_public_key(&self) -> Option<PublicKey> {
        self.validation_public_key
    }

    pub fn network_ops_state(&self) -> Arc<SharedNetworkOpsState> {
        Arc::clone(&self.network_ops_state)
    }

    pub fn ledger_master_state(&self) -> Arc<SharedLedgerMasterState> {
        Arc::clone(&self.ledger_master_state)
    }

    pub fn ledger_master_runtime(&self) -> Option<Arc<AppLedgerMasterRuntime>> {
        self.ledger_master_runtime.as_ref().map(Arc::clone)
    }

    pub fn validations(&self) -> &SharedAppValidations<SystemTimeKeeperClock> {
        &self.validations
    }

    pub fn attach_ledger_master_runtime(
        &mut self,
        ledger_master_runtime: Arc<AppLedgerMasterRuntime>,
    ) -> Option<Arc<AppLedgerMasterRuntime>> {
        if let Some(runtime) = self.network_ops_runtime.as_ref() {
            let _ = runtime.set_ledger_master_runtime(Arc::clone(&ledger_master_runtime));
        }
        let _ = self
            .validations
            .set_ledger_master_runtime(Some(Arc::clone(&ledger_master_runtime)));
        self.ledger_master_runtime.replace(ledger_master_runtime)
    }

    pub fn attach_default_ledger_master_runtime(&mut self) -> Arc<AppLedgerMasterRuntime> {
        if let Some(runtime) = self.ledger_master_runtime() {
            return runtime;
        }

        let runtime = Arc::new(AppLedgerMasterRuntime::default());
        let _ = self.attach_ledger_master_runtime(Arc::clone(&runtime));
        runtime
    }

    pub fn network_ops_runtime(&self) -> Option<Arc<AppNetworkOpsRuntime>> {
        self.network_ops_runtime.as_ref().map(Arc::clone)
    }

    pub fn attach_network_ops_runtime(
        &mut self,
        network_ops_runtime: Arc<AppNetworkOpsRuntime>,
    ) -> Option<Arc<AppNetworkOpsRuntime>> {
        self.network_ops_runtime.replace(network_ops_runtime)
    }

    pub fn network_ops_validation_runtime(&self) -> Option<Arc<AppNetworkOpsValidationRuntime>> {
        self.network_ops_validation_runtime.as_ref().map(Arc::clone)
    }

    pub fn consensus_runtime(&self) -> Option<Arc<AppConsensusRuntime>> {
        self.consensus_runtime.as_ref().map(Arc::clone)
    }

    pub fn attach_network_ops_validation_runtime(
        &mut self,
        network_ops_validation_runtime: Arc<AppNetworkOpsValidationRuntime>,
    ) -> Option<Arc<AppNetworkOpsValidationRuntime>> {
        self.network_ops_validation_runtime
            .replace(network_ops_validation_runtime)
    }

    pub fn attach_default_network_ops_validation_runtime(
        &mut self,
    ) -> Arc<AppNetworkOpsValidationRuntime> {
        if let Some(runtime) = self.network_ops_validation_runtime() {
            return runtime;
        }

        let runtime = Arc::new(AppNetworkOpsValidationRuntime::from_application_root(self));
        let _ = self.attach_network_ops_validation_runtime(Arc::clone(&runtime));
        runtime
    }

    pub fn attach_default_network_ops_runtime(&mut self) -> Arc<AppNetworkOpsRuntime> {
        if let Some(runtime) = self.network_ops_runtime() {
            return runtime;
        }

        let ledger_master_runtime = self.attach_default_ledger_master_runtime();
        let runtime = Arc::new(AppNetworkOpsRuntime::new(
            self.network_ops_state(),
            ledger_master_runtime,
            Arc::clone(&self.registry.hash_router),
            Arc::clone(&self.transaction_master),
            Arc::clone(&self.ledger_master_state),
        ));
        let _ = self.attach_network_ops_runtime(Arc::clone(&runtime));
        runtime
    }

    pub fn network_ops_mode_owner(&self) -> AppNetworkOpsModeOwner {
        let ledger_master_state = Arc::clone(&self.ledger_master_state);
        AppNetworkOpsModeOwner::new(
            self.network_ops_state(),
            Arc::new(move || ledger_master_state.validated_ledger_age()),
        )
    }

    pub fn bind_default_component_runtimes(&mut self) {
        if self.runtime_bindings.nodestore.is_none() {
            if let Some(node_store) = self.registry.node_store.as_ref().cloned() {
                let _ = self.bind_nodestore(Arc::new(AppNodeStoreRuntime::new(node_store)));
            }
        }

        if self.runtime_bindings.ledger.is_none() {
            let ledger_master_runtime = self.attach_default_ledger_master_runtime();
            let network_ops_runtime = self.network_ops_runtime();
            let _ = self.bind_ledger(Arc::new(AppLedgerRuntime::new(
                Arc::clone(&self.registry.ledger_cleaner),
                Arc::clone(&self.registry.inbound_ledgers),
                Arc::clone(&self.registry.inbound_transactions),
                Arc::clone(&self.registry.ledger_replayer),
                ledger_master_runtime,
                network_ops_runtime,
            )));
        }

        if self.runtime_bindings.consensus.is_none() {
            let _ = self.attach_default_consensus_runtime();
        }

        if self.runtime_bindings.validator_site.is_none() {
            self.runtime_bindings.validator_site =
                Some(Arc::new(AppValidatorSiteRuntime::default()));
        }

        if self.runtime_bindings.perf_log.is_none() {
            if let Some(perf_log) = self.registry.perf_log.as_ref().cloned() {
                self.runtime_bindings.perf_log = Some(Arc::new(AppPerfLogRuntime::new(perf_log)));
            }
        }
    }

    pub fn attach_default_consensus_runtime(&mut self) -> Arc<AppConsensusRuntime> {
        if let Some(runtime) = self.consensus_runtime.as_ref() {
            return Arc::clone(runtime);
        }

        let network_ops_runtime = self.attach_default_network_ops_runtime();
        let ledger_master_runtime = self.attach_default_ledger_master_runtime();

        use crate::consensus::rcl_consensus::{
            AppConsensus, AppRclConsensusAdaptor, AppRclConsensusOptions, AppRclConsensusRelay,
            NullRclConsensusJournal,
        };

        let relay = AppRclConsensusRelay::from_application_root(
            self,
            crate::validator::validator_keys::ValidatorKeys::from_sources(None, None),
            NullRclConsensusJournal,
        );

        let adaptor = AppRclConsensusAdaptor::new(
            AppRclConsensusOptions {
                standalone: self.standalone(),
                ..Default::default()
            },
            self.shared_time_keeper(),
            ledger_master_runtime,
            self.registry.open_ledger.clone(),
            self.validations.clone(),
            self.validators.clone(),
            self.network_ops_mode_owner(),
            self.clone_ledger_acceptor(),
            self.registry.inbound_transactions.clone(),
            self.transaction_master.clone(),
            relay,
            NullRclConsensusJournal,
            crate::validator::validator_keys::ValidatorKeys::from_sources(None, None),
            None,
            Some(self.amendment_status.clone()),
        );

        let runner = Box::new(AppConsensus::new(
            adaptor,
            consensus::ConsensusParms::default(),
        ));
        let runtime = Arc::new(AppConsensusRuntime::new(network_ops_runtime));
        runtime.set_runner(runner);

        // to the consensus thread, which calls got_tx_set (event-driven, not polling).
        let (map_complete_tx, map_complete_rx) = std::sync::mpsc::channel();
        self.registry
            .inbound_transactions
            .lock()
            .expect("inbound_transactions mutex")
            .set_map_complete_sender(map_complete_tx);
        runtime.set_map_complete_receiver(map_complete_rx);

        let _ = self.bind_consensus(runtime.clone());
        self.consensus_runtime = Some(runtime.clone());
        runtime
    }

    pub fn submit_transaction_to_network_ops(
        &self,
        transaction: Arc<protocol::STTx>,
    ) -> Option<AppNetworkOpsSubmitReport> {
        let runtime = self.network_ops_runtime.as_ref()?.clone();
        let queued_runtime = Arc::clone(&runtime);
        let job_queue = self.job_queue.clone();

        Some(
            runtime.submit_transaction(transaction, move |queued_transaction| {
                let runtime = Arc::clone(&queued_runtime);
                job_queue.add_job(
                    crate::job::job_types::JobType::Transaction,
                    "SubmitTxn",
                    move || {
                        let mut transaction = Arc::clone(&queued_transaction);
                        let _ = runtime.process_transaction(
                            &mut transaction,
                            false,
                            false,
                            false,
                            || false,
                            || {},
                        );
                    },
                )
            }),
        )
    }

    pub fn network_ops_pending_transaction_count(&self) -> Option<usize> {
        self.network_ops_runtime
            .as_ref()
            .map(|runtime| runtime.pending_transaction_count())
    }

    pub fn network_ops_pending_validation_count(&self) -> Option<usize> {
        self.network_ops_validation_runtime
            .as_ref()
            .map(|runtime| runtime.pending_validation_count())
    }

    pub fn network_ops_submit_held_count(&self) -> Option<usize> {
        self.network_ops_runtime
            .as_ref()
            .map(|runtime| runtime.submit_held_count())
    }

    pub fn promote_included_transaction_to_network_ops(
        &self,
        transaction: &SharedTransaction,
    ) -> Option<usize> {
        self.network_ops_runtime
            .as_ref()
            .map(|runtime| runtime.promote_included_transaction(transaction))
    }

    pub fn apply_held_transactions_to_network_ops(
        &self,
        next_open_ledger_parent_hash: SHAMapHash,
        run_sync_batch: impl FnMut(crate::NetworkOpsProcessSetOwnerSync),
    ) -> Option<AppNetworkOpsApplyHeldOutcome> {
        self.network_ops_runtime.as_ref().map(|runtime| {
            runtime.apply_held_transactions_to_queue(next_open_ledger_parent_hash, run_sync_batch)
        })
    }

    pub fn apply_network_ops_pending_with<RelaySkip>(
        &self,
        current_ledger_index: u32,
        validated_ledger_index: Option<u32>,
        apply_tx: impl FnMut(&SharedTransaction, tx::ApplyFlags) -> tx::ApplyResult,
        report_fee_change: impl FnMut(),
        publish_proposed: impl FnMut(&SharedTransaction, protocol::Ter),
        set_bad_flag: impl FnMut(&SharedTransaction),
        set_held_flag: impl FnMut(&SharedTransaction) -> bool,
        should_relay: impl FnMut(&SharedTransaction) -> Option<RelaySkip>,
        relay: impl FnMut(&SharedTransaction, bool, RelaySkip),
        current_ledger_state: impl FnMut(
            &SharedTransaction,
        ) -> crate::NetworkOpsCurrentLedgerState<
            protocol::XRPAmount,
            u32,
        >,
    ) -> Option<AppNetworkOpsApplyReport> {
        self.network_ops_runtime.as_ref().and_then(|runtime| {
            runtime.apply_pending_with(
                current_ledger_index,
                validated_ledger_index,
                apply_tx,
                report_fee_change,
                publish_proposed,
                set_bad_flag,
                set_held_flag,
                should_relay,
                relay,
                current_ledger_state,
            )
        })
    }

    pub fn apply_network_ops_pending_batch_with<RelaySkip>(
        &self,
        current_ledger_index: u32,
        validated_ledger_index: Option<u32>,
        apply_batch: impl FnOnce(
            &mut [crate::network::network_ops_runtime::AppNetworkOpsPendingTransaction],
        ) -> bool,
        report_fee_change: impl FnMut(),
        publish_proposed: impl FnMut(&SharedTransaction, protocol::Ter),
        set_bad_flag: impl FnMut(&SharedTransaction),
        set_held_flag: impl FnMut(&SharedTransaction) -> bool,
        should_relay: impl FnMut(&SharedTransaction) -> Option<RelaySkip>,
        relay: impl FnMut(&SharedTransaction, bool, RelaySkip),
        current_ledger_state: impl FnMut(
            &SharedTransaction,
        ) -> crate::NetworkOpsCurrentLedgerState<
            protocol::XRPAmount,
            u32,
        >,
    ) -> Option<AppNetworkOpsApplyReport> {
        self.network_ops_runtime.as_ref().and_then(|runtime| {
            runtime.apply_pending_batch_with(
                current_ledger_index,
                validated_ledger_index,
                apply_batch,
                report_fee_change,
                publish_proposed,
                set_bad_flag,
                set_held_flag,
                should_relay,
                relay,
                current_ledger_state,
            )
        })
    }

    pub fn apply_network_ops_pending_to_open_ledger(&self) -> Option<AppNetworkOpsApplyReport> {
        let base_ledger = self.closed_ledger().or_else(|| self.validated_ledger())?;
        let current_ledger_index = self
            .live_current_ledger_index()
            .unwrap_or_else(|| base_ledger.header().seq.saturating_add(1).max(1));
        let validated_ledger_index = self.validated_ledger_seq();
        let tx_q = self.registry.tx_q.clone();
        let open_ledger = self.registry.open_ledger.clone();
        let current_base_fee = self.open_ledger().current().base_fee_drops;
        let state_ledger = Arc::clone(&base_ledger);

        self.apply_network_ops_pending_batch_with(
            current_ledger_index,
            validated_ledger_index,
            move |transactions| {
                let mut changed = false;
                let mut lock = AppTxQLock;
                let mut submit_view = Sandbox::new(Arc::clone(&base_ledger), ApplyFlags::NONE);
                let _ = open_ledger.modify(|view| {
                    for entry in transactions.iter_mut() {
                        let tx = Arc::clone(
                            entry
                                .transaction
                                .lock()
                                .expect("transaction mutex must not be poisoned")
                                .get_s_transaction(),
                        );
                        let tx_source = AppQueueApplyTxSource::new(tx.as_ref());
                        let metrics_snapshot = tx_q.metrics_snapshot();
                        let view_snapshot = view.clone();
                        let live_queue_view = view_snapshot.queue_apply_view(
                            &submit_view,
                            tx.as_ref(),
                            metrics_snapshot,
                        );
                        let queue_view = snapshot_queue_apply_app_view_with_metrics(
                            &tx_source,
                            &live_queue_view,
                            metrics_snapshot,
                        );
                        let submit_rules = submit_view.rules().clone();
                        let preclaim_ter =
                            queue_apply_preclaim_ter(&submit_view, tx.as_ref(), current_ledger_index);
                        let mut runtime = AppOpenLedgerTxQApplyRuntime::new(
                            view,
                            &mut submit_view,
                            Arc::clone(&tx),
                            submit_rules,
                            networkops_apply_flags(entry.admin, entry.fail_hard),
                            current_ledger_index,
                            preclaim_ter,
                        );
                        let result = tx_q
                            .apply_with_owned_metrics_and_derived_preflight_facts_and_hold_admission(
                                &mut lock,
                                &mut runtime,
                                &queue_view,
                                &tx_source,
                            )
                            .apply_result();

                        entry.result = Some(result.ter);
                        entry.applied = result.applied;
                        changed |= result.applied;
                    }
                    changed
                });
                changed
            },
            || {},
            |_tx, _result| {},
            |_tx| {},
            |_tx| false,
            |tx| {
                // reference: hashRouter.shouldRelay(txID) → Optional<set<PeerId>>
                let tx_id = tx
                    .lock()
                    .expect("transaction mutex must not be poisoned")
                    .get_id();
                self.registry.hash_router.should_relay(tx_id)
            },
            |tx, deferred, to_skip| {
                // reference: overlay.relay(txID, tmTransaction, toSkip)
                let Some(overlay_rt) = self.overlay_runtime() else { return };
                let (tx_id, raw_bytes) = {
                    let guard = tx
                        .lock()
                        .expect("transaction mutex must not be poisoned");
                    let stx = guard.get_s_transaction();
                    (guard.get_id(), stx.get_serializer().data().to_vec())
                };
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    .saturating_sub(946684800);
                overlay_rt.overlay().relay_transaction(
                    tx_id,
                    Some(overlay::TmTransaction {
                        raw_transaction: raw_bytes,
                        status: 1, // tsCURRENT
                        receive_timestamp: Some(now),
                        deferred: Some(deferred),
                    }),
                    &to_skip,
                );
            },
            move |transaction| {
                let tx = Arc::clone(
                    transaction
                        .lock()
                        .expect("transaction mutex must not be poisoned")
                        .get_s_transaction(),
                );
                let account = tx.get_account_id(get_field_by_symbol("sfAccount"));
                let account_seq = state_ledger
                    .read(account_keylet(
                        Uint160::from_slice(account.data())
                            .expect("account width should match Uint160"),
                    ))
                    .ok()
                    .flatten()
                    .map(|account_root| account_root.get_field_u32(get_field_by_symbol("sfSequence")))
                    .unwrap_or(0);

                crate::NetworkOpsCurrentLedgerState {
                    fee: protocol::XRPAmount::from_drops(
                        i64::try_from(current_base_fee).unwrap_or(i64::MAX),
                    ),
                    account_seq,
                    available_seq: account_seq,
                }
            },
        )
    }

    pub fn update_local_tx(
        &self,
        ledger: &Ledger,
    ) -> Result<bool, shamap::traversal::TraversalError> {
        let Some(runtime) = self.ledger_master_runtime.as_ref() else {
            return Ok(false);
        };

        runtime.update_local_tx(ledger)?;
        Ok(true)
    }

    pub fn local_tx_count(&self) -> Option<usize> {
        self.ledger_master_runtime
            .as_ref()
            .map(|runtime| runtime.get_local_tx_count())
    }

    pub fn add_held_transaction(&self, transaction: &Transaction) -> bool {
        let Some(runtime) = self.ledger_master_runtime.as_ref() else {
            return false;
        };

        runtime.add_held_transaction(transaction);
        true
    }

    pub fn held_transaction_count(&self) -> Option<usize> {
        self.ledger_master_runtime
            .as_ref()
            .map(|runtime| runtime.held_transaction_count())
    }

    pub fn pop_acct_transaction(&self, transaction: &Transaction) -> Option<Arc<protocol::STTx>> {
        self.ledger_master_runtime
            .as_ref()
            .and_then(|runtime| runtime.pop_acct_transaction_for(transaction))
    }

    pub fn apply_held_transactions<F>(
        &self,
        next_open_ledger_parent_hash: SHAMapHash,
        process_transaction_set: F,
    ) -> Option<usize>
    where
        F: FnMut(CanonicalTXSet),
    {
        self.ledger_master_runtime.as_ref().map(|runtime| {
            runtime.apply_held_transactions(next_open_ledger_parent_hash, process_transaction_set)
        })
    }

    pub fn transaction_master(&self) -> Arc<TransactionMaster> {
        Arc::clone(&self.transaction_master)
    }

    pub fn fetch_cached_transaction(&self, txn_id: &Uint256) -> Option<SharedTransaction> {
        self.transaction_master.fetch_from_cache(txn_id)
    }

    pub fn canonicalize_transaction(&self, txn: &mut SharedTransaction) {
        self.transaction_master.canonicalize(txn);
    }

    pub fn transaction_close_time_seconds(&self, ledger_seq: u32) -> Option<i64> {
        self.ledger_master_state
            .validated_ledger()
            .filter(|ledger| ledger.header().seq == ledger_seq)
            .or_else(|| {
                self.ledger_master_state
                    .published_ledger()
                    .filter(|ledger| ledger.header().seq == ledger_seq)
            })
            .or_else(|| {
                self.ledger_master_state
                    .closed_ledger()
                    .filter(|ledger| ledger.header().seq == ledger_seq)
            })
            .map(|ledger| i64::from(ledger.header().close_time))
    }

    pub fn transaction_json(
        &self,
        transaction: &Transaction,
        options: JsonOptions,
        binary: bool,
    ) -> JsonValue {
        transaction.get_json_with_close_time_source(options, binary, self)
    }

    pub fn validators(&self) -> Arc<ValidatorList> {
        Arc::clone(&self.validators)
    }

    pub fn manifest_cache(&self) -> &Arc<ManifestCache> {
        &self.registry.manifest_cache
    }

    pub fn receive_validation_to_network_ops(
        &self,
        validation: &mut protocol::STValidation,
        source: &str,
    ) -> Option<AppNetworkOpsValidationReceiveReport> {
        self.network_ops_validation_runtime
            .as_ref()
            .map(|runtime| runtime.receive_validation(validation, source))
    }

    pub fn receive_validation_to_network_ops_with_accept(
        &self,
        validation: &mut protocol::STValidation,
        source: &str,
        accept_sink: &dyn crate::RclValidationAcceptanceSink,
    ) -> Option<AppNetworkOpsValidationReceiveReport> {
        self.network_ops_validation_runtime.as_ref().map(|runtime| {
            runtime.receive_validation_with_accept(validation, source, Some(accept_sink))
        })
    }

    pub fn status_rpc_state(&self) -> Arc<StatusRpcState> {
        Arc::clone(&self.status_rpc_state)
    }

    pub fn status_rpc_current_ledger_index(&self) -> Option<u32> {
        self.status_rpc_state.current_ledger_index()
    }

    pub fn set_status_rpc_current_ledger_index(
        &self,
        current_ledger_index: Option<u32>,
    ) -> Option<u32> {
        self.status_rpc_state
            .set_current_ledger_index(current_ledger_index)
    }

    pub fn status_rpc_queue_report(&self) -> Option<QueueTxQRpcReport> {
        self.status_rpc_state.queue_report()
    }

    pub fn set_status_rpc_queue_report(
        &self,
        queue_report: Option<QueueTxQRpcReport>,
    ) -> Option<QueueTxQRpcReport> {
        self.status_rpc_state.set_queue_report(queue_report)
    }

    pub fn status_rpc_peer_count(&self) -> Option<u32> {
        self.status_rpc_state.peer_count()
    }

    pub fn set_status_rpc_peer_count(&self, peer_count: Option<u32>) -> Option<u32> {
        self.status_rpc_state.set_peer_count(peer_count)
    }

    pub fn status_rpc_network_id(&self) -> Option<u32> {
        self.status_rpc_state.network_id()
    }

    pub fn set_status_rpc_network_id(&self, network_id: Option<u32>) -> Option<u32> {
        self.status_rpc_state.set_network_id(network_id)
    }

    pub fn status_rpc_last_close(&self) -> Option<StatusRpcLastClose> {
        self.status_rpc_state.last_close()
    }

    pub fn set_status_rpc_last_close(
        &self,
        last_close: Option<StatusRpcLastClose>,
    ) -> Option<StatusRpcLastClose> {
        self.status_rpc_state.set_last_close(last_close)
    }

    pub fn current_server_time_string(&self) -> String {
        basics::chrono::to_string(OffsetDateTime::now_utc())
    }

    pub fn current_close_time_seconds(&self) -> u32 {
        self.time_keeper.close_time().as_seconds()
    }

    pub fn current_network_time_seconds(&self) -> u32 {
        self.time_keeper.now().as_seconds()
    }

    pub fn close_time_offset_seconds(&self) -> i64 {
        self.time_keeper.close_offset().whole_seconds()
    }

    pub fn status_rpc_hostid(&self) -> Option<String> {
        self.status_rpc_state.hostid()
    }

    pub fn set_status_rpc_hostid(&self, hostid: Option<String>) -> Option<String> {
        self.status_rpc_state.set_hostid(hostid)
    }

    pub fn status_rpc_server_domain(&self) -> Option<String> {
        self.status_rpc_state.server_domain()
    }

    pub fn set_status_rpc_server_domain(&self, server_domain: Option<String>) -> Option<String> {
        self.status_rpc_state.set_server_domain(server_domain)
    }

    pub fn status_rpc_node_size(&self) -> Option<String> {
        self.status_rpc_state.node_size()
    }

    pub fn set_status_rpc_node_size(&self, node_size: Option<String>) -> Option<String> {
        self.status_rpc_state.set_node_size(node_size)
    }

    pub fn status_rpc_io_latency_ms(&self) -> Option<u64> {
        self.status_rpc_state.io_latency_ms()
    }

    pub fn set_status_rpc_io_latency_ms(&self, io_latency_ms: Option<u64>) -> Option<u64> {
        self.status_rpc_state.set_io_latency_ms(io_latency_ms)
    }

    pub fn admin_pubkey_validator(&self) -> String {
        self.validation_public_key
            .and(self.validators.local_public_key())
            .map(|public_key| public_key.to_node_public_base58())
            .unwrap_or_else(|| "none".to_owned())
    }

    pub fn status_rpc_complete_ledgers(&self) -> Option<String> {
        self.status_rpc_state.complete_ledgers()
    }

    pub fn set_status_rpc_complete_ledgers(
        &self,
        complete_ledgers: Option<String>,
    ) -> Option<String> {
        self.status_rpc_state.set_complete_ledgers(complete_ledgers)
    }

    pub fn status_rpc_fetch_pack(&self) -> Option<u32> {
        self.status_rpc_state.fetch_pack()
    }

    pub fn set_status_rpc_fetch_pack(&self, fetch_pack: Option<u32>) -> Option<u32> {
        self.status_rpc_state.set_fetch_pack(fetch_pack)
    }

    pub fn status_rpc_git_info(&self) -> Option<StatusRpcGitInfo> {
        self.status_rpc_state.git_info()
    }

    pub fn validator_list_status_snapshot(&self) -> ValidatorListStatusSnapshot {
        self.validators.status_snapshot()
    }

    pub fn set_status_rpc_git_info(
        &self,
        git_info: Option<StatusRpcGitInfo>,
    ) -> Option<StatusRpcGitInfo> {
        self.status_rpc_state.set_git_info(git_info)
    }

    pub fn set_network_ops_operating_mode(
        &self,
        operating_mode: NetworkOpsOperatingMode,
    ) -> NetworkOpsOperatingMode {
        let previous = self.network_ops_state.operating_mode();
        self.network_ops_state
            .set_operating_mode(normalize_operating_mode_for_validated_age(
                operating_mode,
                self.validated_ledger_age(),
                self.network_ops_state.is_blocked(),
            ));
        let new_mode = self.network_ops_state.operating_mode();
        if previous != new_mode {
            tracing::info!(target: "app", from = %previous.as_str(), to = %new_mode.as_str(), "Operating mode changed");
        }
        previous
    }

    pub fn network_ops_operating_mode(&self) -> NetworkOpsOperatingMode {
        self.network_ops_state.operating_mode()
    }

    pub fn network_ops_operating_mode_string(&self) -> &'static str {
        self.network_ops_state.str_operating_mode()
    }

    pub fn set_need_network_ledger(&self, need_network_ledger: bool) {
        self.network_ops_state
            .set_need_network_ledger(need_network_ledger);
    }

    pub fn need_network_ledger(&self) -> bool {
        self.network_ops_state.need_network_ledger()
    }

    pub fn set_amendment_blocked(&self, amendment_blocked: bool) {
        self.network_ops_state
            .set_amendment_blocked(amendment_blocked);
    }

    pub fn amendment_blocked(&self) -> bool {
        self.network_ops_state.amendment_blocked()
    }

    pub fn set_unl_blocked(&self, unl_blocked: bool) {
        self.network_ops_state.set_unl_blocked(unl_blocked);
    }

    pub fn unl_blocked(&self) -> bool {
        self.network_ops_state.unl_blocked()
    }

    pub fn unsupported_majority_warning_details(
        &self,
    ) -> Option<UnsupportedMajorityWarningDetails> {
        self.amendment_status.unsupported_majority_warning_details()
    }

    pub fn amendment_status(&self) -> Arc<AmendmentStatus> {
        Arc::clone(&self.amendment_status)
    }

    pub fn unsupported_majority_warned(&self) -> bool {
        self.amendment_status.unsupported_majority_warned()
    }

    pub fn set_unsupported_majority_warning_details(
        &self,
        warning: Option<UnsupportedMajorityWarningDetails>,
    ) -> Option<UnsupportedMajorityWarningDetails> {
        self.amendment_status
            .set_unsupported_majority_warning_details(warning)
    }

    pub fn set_unsupported_majority_warned(&self, warned: bool) -> bool {
        self.amendment_status
            .set_unsupported_majority_warned(warned)
    }

    /// Attach the node-store fetcher to a backed `Ledger`.
    ///
    /// fetcher/writer plumbing or re-run state-map setup on every promotion.
    /// Once a ledger already has both seams attached, keep the owner path hot and
    /// return it unchanged.
    pub fn ledger_with_node_fetcher(&self, ledger: Arc<Ledger>) -> Arc<Ledger> {
        let has_shared_family = self.node_family().is_some();
        if (ledger.has_node_fetcher() && ledger.has_node_writer() && !has_shared_family)
            || (!ledger.state_map().backed() && !ledger.tx_map().backed())
        {
            return ledger;
        }

        let fetcher = self.node_fetcher_from_store();
        let writer = self.node_writer_from_store();
        if fetcher.is_none() && writer.is_none() {
            tracing::warn!(target: "ledger",
                "[ledger_fetcher] WARNING: backed ledger seq={} stored without node fetcher/writer \
                 (node store not yet attached) — reads/writes will fail with MissingNode",
                ledger.header().seq
            );
            return ledger;
        }

        let mut ledger_with_fetcher = ledger.as_ref().clone();
        full_sync_debug!(
            "[full_debug][ledger_fetcher] normalize seq={} hash={} account_hash={} tx_hash={} had_fetcher={} had_writer={} shared_family={} backed_state={} backed_tx={}",
            ledger.header().seq,
            ledger.header().hash,
            ledger.header().account_hash,
            ledger.header().tx_hash,
            ledger.has_node_fetcher(),
            ledger.has_node_writer(),
            has_shared_family,
            ledger.state_map().backed(),
            ledger.tx_map().backed()
        );
        if let Some(fetcher) = fetcher
            && (has_shared_family || !ledger_with_fetcher.has_node_fetcher())
        {
            ledger_with_fetcher.set_node_fetcher(fetcher);
        }
        if let Some(writer) = writer
            && !ledger_with_fetcher.has_node_writer()
        {
            ledger_with_fetcher.set_node_writer(writer);
        }
        match ledger_with_fetcher.setup_from_state_map(&feature_xrp_fees()) {
            Ok(true) => {
                full_sync_debug!(
                    "[full_debug][ledger_fetcher] setup seq={} result=loaded fees_base={} fees_reserve={} fees_inc={}",
                    ledger_with_fetcher.header().seq,
                    ledger_with_fetcher.fees().base,
                    ledger_with_fetcher.fees().reserve,
                    ledger_with_fetcher.fees().increment
                );
            }
            Ok(false)
                if ledger_with_fetcher.fees().base == 0
                    || ledger_with_fetcher.fees().reserve == 0
                    || ledger_with_fetcher.fees().increment == 0 =>
            {
                tracing::warn!(target: "ledger",
                    "[ledger_fetcher] WARNING: backed ledger seq={} setup incomplete after fetcher attach",
                    ledger_with_fetcher.header().seq
                );
                full_sync_debug!(
                    "[full_debug][ledger_fetcher] setup seq={} result=incomplete_zero_fee fees_base={} fees_reserve={} fees_inc={}",
                    ledger_with_fetcher.header().seq,
                    ledger_with_fetcher.fees().base,
                    ledger_with_fetcher.fees().reserve,
                    ledger_with_fetcher.fees().increment
                );
            }
            Ok(false) => {
                full_sync_debug!(
                    "[full_debug][ledger_fetcher] setup seq={} result=no_change fees_base={} fees_reserve={} fees_inc={}",
                    ledger_with_fetcher.header().seq,
                    ledger_with_fetcher.fees().base,
                    ledger_with_fetcher.fees().reserve,
                    ledger_with_fetcher.fees().increment
                );
            }
            Err(error) => {
                tracing::warn!(target: "ledger",
                    "[ledger_fetcher] WARNING: backed ledger seq={} setup failed after fetcher attach: {:?}",
                    ledger_with_fetcher.header().seq,
                    error
                );
                full_sync_debug!(
                    "[full_debug][ledger_fetcher] setup seq={} result=error error={:?}",
                    ledger_with_fetcher.header().seq,
                    error
                );
            }
        }
        Arc::new(ledger_with_fetcher)
    }

    pub fn on_closed_ledger(&self, ledger: Arc<Ledger>) {
        self.ledger_master_state
            .note_closed_ledger(self.ledger_with_node_fetcher(ledger));
    }

    pub fn closed_ledger(&self) -> Option<Arc<Ledger>> {
        self.ledger_master_state.closed_ledger()
    }

    pub fn closed_ledger_seq(&self) -> Option<u32> {
        self.ledger_master_state.closed_ledger_seq()
    }

    pub fn on_published_ledger(&self, ledger: Arc<Ledger>) {
        self.ledger_master_state
            .note_published_ledger(self.ledger_with_node_fetcher(ledger));
    }

    pub fn published_ledger(&self) -> Option<Arc<Ledger>> {
        self.ledger_master_state.published_ledger()
    }

    pub fn published_ledger_seq(&self) -> Option<u32> {
        self.ledger_master_state.published_ledger_seq()
    }

    pub fn validated_ledger_age(&self) -> std::time::Duration {
        self.ledger_master_state.validated_ledger_age()
    }

    pub fn is_caught_up(&self) -> LedgerMasterCaughtUp {
        self.ledger_master_state.is_caught_up()
    }

    pub fn server_okay(&self) -> Result<(), &'static str> {
        server_okay(
            self.elb_support,
            &self.stop_tree,
            self.network_ops_state.as_ref(),
            self.is_caught_up(),
            self.load_fee_track.is_loaded_local(),
        )
    }

    pub fn elb_support_enabled(&self) -> bool {
        self.elb_support
    }

    pub fn set_node_identity(
        &mut self,
        node_identity: (PublicKey, SecretKey),
    ) -> Option<(PublicKey, SecretKey)> {
        self.node_identity.replace(node_identity)
    }

    pub fn set_validation_public_key(
        &mut self,
        validation_public_key: PublicKey,
    ) -> Option<PublicKey> {
        self.validation_public_key.replace(validation_public_key)
    }

    pub fn runtime_bindings(&self) -> &RuntimeBindings {
        &self.runtime_bindings
    }

    pub fn set_runtime_bindings(&mut self, bindings: RuntimeBindings) {
        self.runtime_bindings = bindings;
    }

    pub fn shamap_store_service(&self) -> Option<Arc<SHAMapStoreService>> {
        self.shamap_store_service.as_ref().map(Arc::clone)
    }

    pub fn attach_shamap_store_service(
        &mut self,
        service: Arc<SHAMapStoreService>,
    ) -> Option<Arc<SHAMapStoreService>> {
        let handle: ManagedHandle = service.clone();
        self.runtime_bindings.shamap_store = Some(handle);
        self.shamap_store_service.replace(service)
    }

    pub fn attach_shamap_store_component(
        &mut self,
        component: Arc<SHAMapStoreComponent>,
    ) -> Arc<SHAMapStoreService> {
        let service = Arc::new(SHAMapStoreService::new(
            component,
            Arc::new(crate::SharedSHAMapStoreHealthState::new_with_app_state(
                self.time_keeper.clone(),
                self.network_ops_state(),
                self.ledger_master_state(),
            )),
        ));
        let _ = self.attach_shamap_store_service(Arc::clone(&service));
        service
    }

    pub fn bind_ledger(&mut self, component: ManagedHandle) -> Option<ManagedHandle> {
        self.runtime_bindings.ledger.replace(component)
    }

    pub fn bind_nodestore(&mut self, component: ManagedHandle) -> Option<ManagedHandle> {
        self.runtime_bindings.nodestore.replace(component)
    }

    pub fn bind_shamap_store(&mut self, component: ManagedHandle) -> Option<ManagedHandle> {
        self.shamap_store_service = None;
        self.runtime_bindings.shamap_store.replace(component)
    }

    pub fn bind_overlay(&mut self, component: ManagedHandle) -> Option<ManagedHandle> {
        self.overlay_runtime = None;
        self.runtime_bindings.overlay.replace(component)
    }

    pub fn bind_consensus(&mut self, component: ManagedHandle) -> Option<ManagedHandle> {
        self.runtime_bindings.consensus.replace(component)
    }

    pub fn bind_server(&mut self, component: ManagedHandle) -> Option<ManagedHandle> {
        self.runtime_bindings.server.replace(component)
    }

    pub fn bind_grpc(&mut self, component: ManagedHandle) {
        self.runtime_bindings.grpc = GrpcRuntime::Enabled(component);
    }

    pub fn disable_grpc(&mut self, reason: impl Into<String>) {
        self.runtime_bindings.grpc = GrpcRuntime::DisabledExplicit {
            reason: reason.into(),
        };
    }

    pub fn register_stop_callback(
        &self,
        name: impl Into<String>,
        callback: impl Fn() + Send + Sync + 'static,
    ) -> Arc<StopTreeNode> {
        self.stop_tree.register_callback(name, callback)
    }

    pub fn wire_node_family_reset(&self) -> Option<Arc<StopTreeNode>> {
        let node_family = self.node_family()?;
        Some(
            self.stop_tree
                .register_callback("node-family-reset", move || {
                    node_family.reset();
                }),
        )
    }

    pub fn set_shamap_store_operating_mode(
        &self,
        operating_mode: SHAMapStoreOperatingMode,
    ) -> bool {
        let Some(service) = self.shamap_store_service.as_ref() else {
            return false;
        };
        self.network_ops_state
            .set_operating_mode(match operating_mode {
                SHAMapStoreOperatingMode::Full => NetworkOpsOperatingMode::Full,
                SHAMapStoreOperatingMode::Other => NetworkOpsOperatingMode::Connected,
            });
        service.set_operating_mode(operating_mode);
        true
    }

    pub fn shamap_store_operating_mode(&self) -> Option<SHAMapStoreOperatingMode> {
        self.shamap_store_service
            .as_ref()
            .map(|service| service.operating_mode())
    }

    pub fn on_validated_ledger(&self, ledger: Arc<Ledger>) -> bool {
        let ledger = self.ledger_with_node_fetcher(ledger);
        self.ledger_master_state
            .note_validated_ledger(Arc::clone(&ledger));
        self.amendment_status.do_validated_ledger(ledger.as_ref());
        if !self.network_ops_state.is_blocked() {
            if self.amendment_status.has_unsupported_enabled() {
                self.network_ops_state.set_amendment_blocked(true);
            } else {
                self.amendment_status
                    .sync_warning_state_for_validated_ledger(ledger.as_ref());
            }
        }
        let Some(service) = self.shamap_store_service.as_ref() else {
            return true;
        };
        service.on_ledger_closed(ledger);
        true
    }

    /// Records the app-visible validated ledger without running heavier
    /// validated-ledger side effects.
    ///
    /// before it can publish available ledgers. The full `on_validated_ledger`
    /// path remains the parity target for those hooks once they are safe to run
    /// outside the catchup hot path.
    pub fn note_validated_ledger_for_sync(&self, ledger: Arc<Ledger>) {
        self.ledger_master_state
            .note_validated_ledger(self.ledger_with_node_fetcher(ledger));
    }

    pub fn validated_ledger(&self) -> Option<Arc<Ledger>> {
        self.ledger_master_state.validated_ledger()
    }

    pub fn validated_ledger_seq(&self) -> Option<u32> {
        self.ledger_master_state.validated_ledger_seq()
    }

    pub fn accept_standalone_ledger(&self) -> Result<u32, String> {
        if !self.standalone() {
            return Err("ledger_accept requires standalone mode".to_owned());
        }

        let current = self.open_ledger().current();
        let closed_seq = current
            .ledger_current_index
            .max(self.closed_ledger_seq().unwrap_or(0).saturating_add(1))
            .max(self.validated_ledger_seq().unwrap_or(0).saturating_add(1))
            .max(1);
        let closed_seq = closed_seq.max(1);
        let close_time = self.current_close_time_seconds();

        self.accept_ledger(closed_seq, close_time, current.base_fee_drops)
    }

    pub fn accept_ledger(
        &self,
        closed_seq: u32,
        close_time: u32,
        base_fee_drops: u64,
    ) -> Result<u32, String> {
        let parent_ledger = self.closed_ledger().or_else(|| self.validated_ledger());
        let current_rules = parent_ledger
            .as_ref()
            .map(|ledger| ledger.rules().clone())
            .unwrap_or_else(|| Rules::new(std::iter::empty::<Uint256>()));
        let accept_journal = self.registry.logs.journal("accept_ledger");
        let next_open_parent_hash = self
            .closed_ledger()
            .or_else(|| self.validated_ledger())
            .map(|ledger| ledger.header().hash)
            .unwrap_or_default();

        let _ = self.apply_held_transactions_to_network_ops(next_open_parent_hash, |_sync| {});
        let mut accepted_entries = Vec::new();

        // Create a mutable view on the parent ledger to accumulate state changes
        let state_view_base = parent_ledger
            .clone()
            .unwrap_or_else(|| Arc::new(Ledger::from_ledger_seq_and_close_time(1, 0, false)));
        let state_view = std::sync::Mutex::new(ledger::ApplyViewImpl::new(
            Arc::clone(&state_view_base),
            protocol::ApplyFlags::NONE,
        ));

        if let Some(report) = self.apply_network_ops_pending_with(
            closed_seq,
            self.validated_ledger_seq(),
            {
                let _current_rules = current_rules.clone();
                let _parent_ledger = parent_ledger.clone();
                let _accept_journal = Arc::clone(&accept_journal);
                let state_view_ref = &state_view;
                move |tx, _flags| {
                    // Run the REAL transactor on the shared state view
                    let sttx = tx
                        .lock()
                        .expect("transaction mutex must not be poisoned")
                        .get_s_transaction()
                        .clone();
                    let txn_type = sttx.get_txn_type();
                    let mut view = state_view_ref
                        .lock()
                        .expect("state view mutex must not be poisoned");
                    let result = handle_real_dispatch(&mut *view, &sttx, txn_type, None);
                    let applied =
                        protocol::is_tes_success(result) || protocol::is_tec_claim(result);
                    tx::ApplyResult::new(result, applied, false)
                }
            },
            || {},
            |_tx, _result| {},
            |_tx| {},
            |_tx| false,
            |_tx| None::<()>,
            |_tx, _deferred, _skip| {},
            |_tx| crate::NetworkOpsCurrentLedgerState {
                fee: protocol::XRPAmount::from_drops(base_fee_drops as i64),
                account_seq: 0_u32,
                available_seq: 0_u32,
            },
        ) {
            for (index, entry) in report.entries.iter().enumerate() {
                if entry.applied {
                    let _ = self.transaction_master.in_ledger(
                        entry.transaction_id,
                        closed_seq,
                        Some(index as u32),
                        Some(self.registry.network_id_service.get_network_id()),
                    );

                    if let Some(shared_tx) = self
                        .transaction_master
                        .fetch_from_cache(&entry.transaction_id)
                    {
                        if let Ok(tx) = shared_tx.lock() {
                            let mut meta = protocol::TxMeta::new(entry.transaction_id, closed_seq);
                            let mut serializer = protocol::Serializer::default();
                            meta.add_raw(&mut serializer, entry.result, index as u32);
                            accepted_entries.push(StandaloneAcceptedTx {
                                transaction_id: entry.transaction_id,
                                txn: Arc::new(protocol::Serializer::from_bytes(
                                    tx.get_s_transaction().get_serializer().data(),
                                )),
                                metadata: Arc::new(serializer),
                            });
                        }
                    }
                }
            }
        }

        let closed = match parent_ledger {
            Some(parent) if parent.header().seq.saturating_add(1) == closed_seq => {
                crate::build_ledger_from_view(
                    Arc::clone(&parent),
                    close_time,
                    true,
                    0,
                    accept_journal.as_ref(),
                    |built| {
                        StandaloneLedgerBuildView::from_base(
                            Arc::new(built.clone()),
                            &accepted_entries,
                        )
                    },
                    |_ledger| 0,
                    |_ledger| 0,
                    |_ledger| {},
                )
                .map_err(|error| {
                    tracing::error!(target: "app", "Failed to apply state to closed ledger");
                    format!("standalone ledger build failed: {error:?}")
                })?
            }
            Some(parent) => {
                return Err(format!(
                    "standalone accept expected next sequence {} but received {}",
                    parent.header().seq.saturating_add(1),
                    closed_seq
                ));
            }
            None => {
                let mut closed =
                    Ledger::from_ledger_seq_and_close_time(closed_seq, close_time, false);
                crate::BuildLedgerView::apply_to_ledger(
                    StandaloneLedgerBuildView::from_base(
                        Arc::new(closed.clone()),
                        &accepted_entries,
                    ),
                    &mut closed,
                );
                closed.set_accepted(close_time, 0, true);
                Arc::new(closed)
            }
        };

        // Apply accumulated state changes from the transactor to the closed ledger
        let closed = {
            let view = state_view
                .into_inner()
                .expect("state view mutex must not be poisoned");
            let mut ledger = Arc::try_unwrap(closed).unwrap_or_else(|arc| (*arc).clone());
            let _ = view.table().apply(&mut ledger);
            Arc::new(ledger)
        };

        let tx_count = accepted_entries.len();
        tracing::info!(target: "app", seq = closed_seq, tx_count, close_time, "Ledger closed");

        let _ = self.process_closed_ledger_txq(closed.as_ref(), false);
        self.on_closed_ledger(Arc::clone(&closed));
        self.on_published_ledger(Arc::clone(&closed));
        let _ = self.on_validated_ledger(Arc::clone(&closed));

        use ledger::LedgerPersistenceRuntime;
        self.build_ledger_persistence_runtime()
            .save_validated_ledger(Arc::clone(&closed), true);

        let next_open_index = closed_seq.saturating_add(1);
        self.rebuild_open_ledger_after_close(
            next_open_index,
            base_fee_drops,
            *closed.header().hash.as_uint256(),
        );
        self.set_status_rpc_current_ledger_index(Some(next_open_index));
        self.set_status_rpc_queue_report(Some(self.tx_q_rpc_report()));

        Ok(next_open_index)
    }

    pub fn is_shamap_store_stopping(&self) -> Option<bool> {
        self.shamap_store_service
            .as_ref()
            .map(|service| service.is_stopping())
    }

    pub fn signal_stop(&self, reason: impl Into<String>) -> bool {
        tracing::info!(target: "app", "Node shutting down");
        self.stop_tree.signal_stop(reason)
    }

    pub fn is_stopping(&self) -> bool {
        self.stop_tree.is_stopping()
    }

    pub fn stop_reason(&self) -> Option<String> {
        self.stop_tree.reason()
    }
}

impl TransactionCloseTimeSource for ApplicationRoot {
    fn close_time_for_ledger_seq(&self, ledger_seq: u32) -> Option<i64> {
        self.transaction_close_time_seconds(ledger_seq)
    }
}

impl QueueTxQClosedLedgerAppSource<AppClosedLedgerTxQView<'_>> for ApplicationRoot {
    fn validated_fee_levels(&self, view: &AppClosedLedgerTxQView<'_>) -> Vec<u64> {
        self.validated_fee_levels_for_closed_ledger(view.ledger)
    }
}

impl ServiceRegistry for ApplicationRoot {
    type CollectorManager = CollectorManager;
    type NodeFamily = Option<Arc<dyn NodeFamilyRuntime>>;
    type TimeKeeper = Arc<TimeKeeper<SystemTimeKeeperClock>>;
    type JobQueue = JobQueue;
    type TempNodeCache = Arc<shamap::tree_node_cache::TreeNodeCache>;
    type CachedSles = Arc<ledger::CachedSles>;
    type NetworkIdService = FixedNetworkIdService;
    type AmendmentTable = Arc<AmendmentStatus>;
    type HashRouter = Arc<HashRouter>;
    type LoadFeeTrack = Arc<SharedLoadFeeTrack>;
    type LoadManager = LoadManager;
    type Validations = SharedAppValidations<SystemTimeKeeperClock>;
    type ValidatorList = Arc<ValidatorList>;
    type ValidatorSite = ValidatorSite;
    type ManifestCache = ManifestCache;
    type Overlay = Option<Arc<dyn OverlayStatusSource>>;
    type Cluster = Cluster;
    type PeerReservationTable = PeerReservationTable<PublicKey>;
    type ResourceManager = Arc<resource::ResourceManager>;
    type NodeStore = Option<crate::shamap::shamap_store_backend::SHAMapStoreNodeStore>;
    type ShamapStore = Option<Arc<SHAMapStoreService>>;
    type RelationalDatabase =
        Option<Arc<crate::shamap::shamap_store_relational::SqliteSHAMapStoreRelational>>;
    type InboundLedgers = AppInboundLedgers;
    type InboundTransactions = AppInboundTransactions;
    type AcceptedLedgerCache = AppAcceptedLedgerCache;
    type LedgerMaster = Arc<SharedLedgerMasterState>;
    type LedgerCleaner = Arc<ledger::LedgerCleaner>;
    type LedgerReplayer = Arc<Mutex<ledger::LedgerReplayer>>;
    type PendingSaves = Arc<ledger::PendingSaves>;
    type OpenLedger = SharedAppOpenLedger;
    type NetworkOps = Arc<SharedNetworkOpsState>;
    type OrderBookDb = Arc<ledger::OrderBookDB>;
    type TransactionMaster = Arc<TransactionMaster>;
    type TxQ = SharedAppTxQ;
    type PathRequestManager = Arc<crate::paths::PathRequestManager>;
    type ServerHandler = Arc<AppServerHandler>;
    type PerfLog = Arc<PerfLogImp>;
    type Journal = Arc<crate::state::app_registry::AppJournal>;
    type IoContext = RuntimeBindings;
    type Config = AppConfig;
    type Logs = Arc<AppLogs>;
    type TrapTxId = Uint256;
    type WalletDb = Arc<DatabaseCon>;
    type Application = ApplicationRoot;

    fn get_collector_manager(&self) -> &Self::CollectorManager {
        &self.collector_manager
    }

    fn get_node_family(&self) -> &Self::NodeFamily {
        &self.node_family
    }

    fn get_time_keeper(&self) -> &Self::TimeKeeper {
        &self.time_keeper
    }

    fn get_job_queue(&self) -> &Self::JobQueue {
        &self.job_queue
    }

    fn get_temp_node_cache(&self) -> &Self::TempNodeCache {
        &self.registry.temp_node_cache
    }

    fn get_cached_sles(&self) -> &Self::CachedSles {
        &self.registry.cached_sles
    }

    fn get_network_id_service(&self) -> &Self::NetworkIdService {
        &self.registry.network_id_service
    }

    fn get_amendment_table(&self) -> &Self::AmendmentTable {
        &self.amendment_status
    }

    fn get_hash_router(&self) -> &Self::HashRouter {
        &self.registry.hash_router
    }

    fn get_fee_track(&self) -> &Self::LoadFeeTrack {
        &self.load_fee_track
    }

    fn get_load_manager(&self) -> &Self::LoadManager {
        &self.load_manager
    }

    fn get_validations(&self) -> &Self::Validations {
        &self.validations
    }

    fn get_validators(&self) -> &Self::ValidatorList {
        &self.validators
    }

    fn get_validator_sites(&self) -> &Self::ValidatorSite {
        &self.registry.validator_sites
    }

    fn get_validator_manifests(&self) -> &Self::ManifestCache {
        &self.registry.manifest_cache
    }

    fn get_publisher_manifests(&self) -> &Self::ManifestCache {
        &self.registry.manifest_cache
    }

    fn get_overlay(&self) -> &Self::Overlay {
        &self.overlay_status
    }

    fn get_cluster(&self) -> &Self::Cluster {
        self.registry.cluster.as_ref()
    }

    fn get_peer_reservations(&self) -> &Self::PeerReservationTable {
        self.registry.peer_reservations.as_ref()
    }

    fn get_resource_manager(&self) -> &Self::ResourceManager {
        &self.registry.resource_manager
    }

    fn get_node_store(&self) -> &Self::NodeStore {
        &self.registry.node_store
    }

    fn get_shamap_store(&self) -> &Self::ShamapStore {
        &self.shamap_store_service
    }

    fn get_relational_database(&self) -> &Self::RelationalDatabase {
        &self.registry.relational_database
    }

    fn get_inbound_ledgers(&self) -> &Self::InboundLedgers {
        &self.registry.inbound_ledgers
    }

    fn get_inbound_transactions(&self) -> &Self::InboundTransactions {
        &self.registry.inbound_transactions
    }

    fn get_accepted_ledger_cache(&self) -> &Self::AcceptedLedgerCache {
        &self.registry.accepted_ledger_cache
    }

    fn get_ledger_master(&self) -> &Self::LedgerMaster {
        &self.ledger_master_state
    }

    fn get_ledger_cleaner(&self) -> &Self::LedgerCleaner {
        &self.registry.ledger_cleaner
    }

    fn get_ledger_replayer(&self) -> &Self::LedgerReplayer {
        &self.registry.ledger_replayer
    }

    fn get_pending_saves(&self) -> &Self::PendingSaves {
        &self.registry.pending_saves
    }

    fn get_open_ledger(&self) -> &Self::OpenLedger {
        &self.registry.open_ledger
    }

    fn get_open_ledger_const(&self) -> &Self::OpenLedger {
        &self.registry.open_ledger
    }

    fn get_ops(&self) -> &Self::NetworkOps {
        &self.network_ops_state
    }

    fn get_order_book_db(&self) -> &Self::OrderBookDb {
        &self.registry.order_book_db
    }

    fn get_master_transaction(&self) -> &Self::TransactionMaster {
        &self.transaction_master
    }

    fn get_tx_q(&self) -> &Self::TxQ {
        &self.registry.tx_q
    }

    fn get_path_request_manager(&self) -> &Self::PathRequestManager {
        &self.registry.path_request_manager
    }

    fn get_server_handler(&self) -> &Self::ServerHandler {
        &self.registry.server_handler
    }

    fn get_perf_log(&self) -> &Self::PerfLog {
        self.registry
            .perf_log
            .as_ref()
            .expect("application root must own a perf log")
    }

    fn is_stopping(&self) -> bool {
        self.stop_tree.is_stopping()
    }

    fn get_journal(&self, name: &str) -> Self::Journal {
        self.registry.logs.journal(name)
    }

    fn get_io_context(&self) -> &Self::IoContext {
        &self.runtime_bindings
    }

    fn get_config(&self) -> &Self::Config {
        &self.registry.config
    }

    fn get_logs(&self) -> &Self::Logs {
        &self.registry.logs
    }

    fn get_trap_tx_id(&self) -> &Option<Self::TrapTxId> {
        &self.registry.trap_tx_id
    }

    fn get_wallet_db(&self) -> &Self::WalletDb {
        &self.registry.wallet_db
    }

    fn get_app(&self) -> &Self::Application {
        self
    }
}

#[cfg(test)]
mod tests;
