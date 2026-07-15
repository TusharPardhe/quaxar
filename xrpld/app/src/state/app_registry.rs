use crate::BuildLedgerJournal;
use crate::consensus::rcl_validations::RclValidationJournal;
use crate::ledger::open_ledger::{OpenLedger, OpenLedgerTx, OpenLedgerView};
use crate::load::load_manager::LoadManagerJournal;
use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;
use crate::shamap::shamap_store_relational::SqliteSHAMapStoreRelational;
use crate::state::manifest::ManifestCache;
use crate::tx_queue::txq::TxQ;
use crate::validator::validator_site::ValidatorSite;
use basics::base_uint::Uint256;
use basics::tagged_cache::{MonotonicClock, TaggedCache};
use ledger::{
    AcceptedLedger, CachedSles, InboundLedgersLocal, InboundTransactions, LedgerCleaner,
    LedgerReplayer, OrderBookDB, PendingSaves, ReadView,
};
use nodestore::{JournalLevel as NodeStoreJournalLevel, NodeStoreJournal};
use overlay::Cluster;
use overlay::SimplePeerSetBuilder;
use perflog::{
    JournalLevel as PerfLogJournalLevel, PerfLogImp, PerfLogJournal, PerfLogReportSource,
    PerfLogSetup,
};

/// Minimal `PerfLogReportSource` that emits all five reference state-accounting keys
/// so `server_info` / `server_state` structural parity holds.
struct StateAccountingReportSource {
    network_ops_state: Arc<Mutex<Option<Arc<crate::network::network_ops::SharedNetworkOpsState>>>>,
}

impl PerfLogReportSource for StateAccountingReportSource {
    fn node_store_counts_json(&self) -> serde_json::Value {
        serde_json::Value::Object(serde_json::Map::new())
    }

    fn state_accounting(&self, report: &mut serde_json::Value) {
        let state_lock = self
            .network_ops_state
            .lock()
            .expect("network_ops_state mutex poisoned");

        if let Some(state) = state_lock.as_ref() {
            if let serde_json::Value::Object(map) = report {
                map.insert("state_accounting".to_owned(), state.state_accounting_json());
                map.insert(
                    "server_state_duration_us".to_owned(),
                    serde_json::Value::String(state.server_state_duration_us()),
                );
                if let Some(initial_sync) = state.initial_sync_duration_us() {
                    map.insert(
                        "initial_sync_duration_us".to_owned(),
                        serde_json::Value::String(initial_sync),
                    );
                }
            }
        } else {
            let zero_entry = serde_json::json!({
                "duration_us": "0",
                "transitions": "0"
            });
            let accounting = serde_json::json!({
                "connected":    zero_entry.clone(),
                "disconnected": zero_entry.clone(),
                "full":         zero_entry.clone(),
                "syncing":      zero_entry.clone(),
                "tracking":     zero_entry,
            });
            if let serde_json::Value::Object(map) = report {
                map.insert("state_accounting".to_owned(), accounting);
                map.entry("server_state_duration_us")
                    .or_insert_with(|| serde_json::Value::String("0".to_owned()));
            }
        }
    }
}
use protocol::{AccountID, PublicKey};
use resource::ResourceManager;
use shamap::tree_node_cache::TreeNodeCache;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use time::Duration as TimeDuration;
use tx::{
    QueueAcceptLockScope, QueueAcceptOwnerState, QueueApplyHoldPreflightTxSource,
    QueueApplyLockScope, QueueApplyObservedAccountLookup, QueueApplyObservedTicketLookup,
    QueueApplyObservedTxSource, QueueApplyObservedViewSource, QueueFeeMetricsSnapshot,
    QueueTxQMetricsView, QueueTxQRpcView, QueueViews, TxQSetup,
};
use xrpl_core::{
    FixedNetworkIdService, HashRouter, HashRouterSetup, LoadMonitorJournal,
    LoadMonitorJournalFactory, PeerReservationJournal, PeerReservationTable,
};
use xrpld_core::{DatabaseCon, WALLET_DB_INIT, WALLET_DB_NAME};

static WALLET_DB_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub type AppInboundLedgers = Arc<Mutex<InboundLedgersLocal<MonotonicClock>>>;
pub type AppInboundTransactions = Arc<Mutex<InboundTransactions>>;
pub type AppAcceptedLedgerCache = Arc<TaggedCache<Uint256, Arc<AcceptedLedger>>>;
pub type AppTxQAccount = AccountID;
pub type AppTxQTransaction = Arc<protocol::STTx>;
pub type AppTxQJournalTag = String;
pub type AppTxQParentBatchId = String;
pub type AppTxQ = TxQ<AppTxQAccount, AppTxQTransaction, AppTxQJournalTag, AppTxQParentBatchId>;

pub const APP_OPEN_LEDGER_DEFAULT_BASE_FEE_DROPS: u64 = 10;

fn raw_account_id(account: AccountID) -> basics::base_uint::Uint160 {
    basics::base_uint::Uint160::from_slice(account.data())
        .expect("AccountID width should match Uint160")
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppPlaceholder;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppServerHandlerState {
    pub configured_ports: Vec<String>,
    pub deferred_protocols: Vec<String>,
    pub started: bool,
}

#[derive(Debug, Default)]
pub struct AppServerHandler {
    state: Mutex<AppServerHandlerState>,
}

impl AppServerHandler {
    pub fn snapshot(&self) -> AppServerHandlerState {
        self.state
            .lock()
            .expect("app server handler mutex must not be poisoned")
            .clone()
    }

    pub fn configure(
        &self,
        configured_ports: Vec<String>,
        deferred_protocols: Vec<String>,
    ) -> AppServerHandlerState {
        let mut state = self
            .state
            .lock()
            .expect("app server handler mutex must not be poisoned");
        let previous = state.clone();
        state.configured_ports = configured_ports;
        state.deferred_protocols = deferred_protocols;
        previous
    }

    pub fn mark_started(&self, started: bool) -> bool {
        let mut state = self
            .state
            .lock()
            .expect("app server handler mutex must not be poisoned");
        std::mem::replace(&mut state.started, started)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppOpenLedgerTxRecord {
    pub tx: std::sync::Arc<protocol::STTx>,
}

impl AppOpenLedgerTxRecord {
    pub fn new(tx: std::sync::Arc<protocol::STTx>) -> Self {
        Self { tx }
    }
}

impl OpenLedgerTx for AppOpenLedgerTxRecord {
    type Id = Uint256;

    fn tx_id(&self) -> Self::Id {
        self.tx.get_transaction_id()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AppQueueApplyTxSource<'a> {
    tx: &'a protocol::STTx,
    account: AccountID,
    transaction_id: Uint256,
    tx_seq_proxy: protocol::SeqProxy,
}

impl<'a> AppQueueApplyTxSource<'a> {
    pub fn new(tx: &'a protocol::STTx) -> Self {
        Self {
            tx,
            account: tx.get_account_id(protocol::get_field_by_symbol("sfAccount")),
            transaction_id: tx.get_transaction_id(),
            tx_seq_proxy: tx.get_seq_proxy(),
        }
    }

    pub const fn tx(&self) -> &'a protocol::STTx {
        self.tx
    }
}

impl QueueApplyObservedTxSource for AppQueueApplyTxSource<'_> {
    type Account = AccountID;
    type TransactionId = Uint256;

    fn account(&self) -> &Self::Account {
        &self.account
    }

    fn transaction_id(&self) -> Self::TransactionId {
        self.transaction_id
    }

    fn tx_id(&self) -> Uint256 {
        self.transaction_id
    }

    fn tx_seq_proxy(&self) -> protocol::SeqProxy {
        self.tx_seq_proxy
    }
}

impl QueueApplyHoldPreflightTxSource for AppQueueApplyTxSource<'_> {
    fn has_previous_txn_id(&self) -> bool {
        self.tx
            .is_field_present(protocol::get_field_by_symbol("sfPreviousTxnID"))
    }

    fn has_account_txn_id(&self) -> bool {
        self.tx
            .is_field_present(protocol::get_field_by_symbol("sfAccountTxnID"))
    }

    fn last_valid_ledger(&self) -> Option<u32> {
        let field = protocol::get_field_by_symbol("sfLastLedgerSequence");
        self.tx
            .is_field_present(field)
            .then(|| self.tx.get_field_u32(field))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppOpenLedgerView {
    pub ledger_current_index: u32,
    pub base_fee_drops: u64,
    pub parent_hash: Uint256,
    txs: Vec<AppOpenLedgerTxRecord>,
}

impl Default for AppOpenLedgerView {
    fn default() -> Self {
        Self::new(0, APP_OPEN_LEDGER_DEFAULT_BASE_FEE_DROPS)
    }
}

impl AppOpenLedgerView {
    pub fn new(ledger_current_index: u32, base_fee_drops: u64) -> Self {
        Self::with_parent_hash(ledger_current_index, base_fee_drops, Uint256::zero())
    }

    pub fn with_parent_hash(
        ledger_current_index: u32,
        base_fee_drops: u64,
        parent_hash: Uint256,
    ) -> Self {
        Self {
            ledger_current_index,
            base_fee_drops,
            parent_hash,
            txs: Vec::new(),
        }
    }

    pub fn with_transactions(
        ledger_current_index: u32,
        base_fee_drops: u64,
        parent_hash: Uint256,
        txs: Vec<AppOpenLedgerTxRecord>,
    ) -> Self {
        Self {
            ledger_current_index,
            base_fee_drops,
            parent_hash,
            txs,
        }
    }

    pub fn push_transaction(&mut self, tx: std::sync::Arc<protocol::STTx>) {
        self.txs.push(AppOpenLedgerTxRecord::new(tx));
    }

    pub fn tx_ids(&self) -> Vec<Uint256> {
        self.txs.iter().map(|tx| tx.tx_id()).collect()
    }

    pub fn queue_apply_view<'a, V>(
        &'a self,
        read_view: &'a V,
        tx: &'a protocol::STTx,
        metrics_snapshot: QueueFeeMetricsSnapshot,
    ) -> AppQueueApplyLedgerView<'a, V>
    where
        V: ReadView,
    {
        AppQueueApplyLedgerView::new(self, read_view, tx, metrics_snapshot)
    }
}

impl OpenLedgerView<AppOpenLedgerTxRecord> for AppOpenLedgerView {
    fn tx_count(&self) -> usize {
        self.txs.len()
    }

    fn ordered_txs(&self) -> Vec<AppOpenLedgerTxRecord> {
        self.txs.clone()
    }
}

impl tx::QueueAcceptLedgerViewSource for AppOpenLedgerView {
    fn open_ledger_tx_count(&self) -> usize {
        self.txs.len()
    }

    fn parent_hash(&self) -> Uint256 {
        self.parent_hash
    }
}

impl QueueTxQRpcView for AppOpenLedgerView {
    fn ledger_current_index(&self) -> u32 {
        self.ledger_current_index
    }

    fn open_ledger_tx_count(&self) -> usize {
        self.txs.len()
    }

    fn base_fee_drops(&self) -> u64 {
        self.base_fee_drops
    }
}

#[derive(Debug, Clone)]
pub struct AppQueueApplyLedgerView<'a, V> {
    open_ledger: &'a AppOpenLedgerView,
    read_view: &'a V,
    rules: protocol::Rules,
    calculated_base_fee_drops: i64,
    fee_paid_drops: i64,
    default_base_fee_drops: i64,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    reserve_drops: u64,
    base_fee_drops: u64,
}

impl<'a, V> AppQueueApplyLedgerView<'a, V>
where
    V: ReadView,
{
    pub fn new(
        open_ledger: &'a AppOpenLedgerView,
        read_view: &'a V,
        tx: &'a protocol::STTx,
        metrics_snapshot: QueueFeeMetricsSnapshot,
    ) -> Self {
        let base_fee_drops = read_view.fees().base;
        let fee_field = protocol::get_field_by_symbol("sfFee");
        let fee_paid_drops = if tx.is_field_present(fee_field) {
            tx.get_field_amount(fee_field).xrp().drops()
        } else {
            0
        };

        Self {
            open_ledger,
            read_view,
            rules: read_view.rules(),
            calculated_base_fee_drops: i64::try_from(base_fee_drops).unwrap_or(i64::MAX),
            fee_paid_drops,
            default_base_fee_drops: i64::try_from(base_fee_drops).unwrap_or(i64::MAX),
            metrics_snapshot,
            reserve_drops: read_view.fees().account_reserve(0),
            base_fee_drops,
        }
    }
}

impl<V> QueueApplyObservedViewSource<AccountID> for AppQueueApplyLedgerView<'_, V>
where
    V: ReadView,
{
    fn rules(&self) -> &protocol::Rules {
        &self.rules
    }

    fn account_lookup(&self, account: &AccountID) -> QueueApplyObservedAccountLookup {
        let sequence_field = protocol::get_field_by_symbol("sfSequence");
        let balance_field = protocol::get_field_by_symbol("sfBalance");
        match self
            .read_view
            .read(protocol::account_keylet(raw_account_id(*account)))
        {
            Ok(Some(account_root)) => QueueApplyObservedAccountLookup::Present {
                sequence: account_root.get_field_u32(sequence_field),
                balance_drops: u64::try_from(
                    account_root.get_field_amount(balance_field).xrp().drops(),
                )
                .unwrap_or(0),
            },
            Ok(None) | Err(_) => QueueApplyObservedAccountLookup::Missing,
        }
    }

    fn ticket_lookup(
        &self,
        account: &AccountID,
        tx_seq_proxy: protocol::SeqProxy,
    ) -> QueueApplyObservedTicketLookup {
        if !tx_seq_proxy.is_ticket() {
            return QueueApplyObservedTicketLookup::NotRequired;
        }

        match self
            .read_view
            .exists(protocol::ticket_keylet_from_seq_proxy(
                raw_account_id(*account),
                tx_seq_proxy,
            )) {
            Ok(true) => QueueApplyObservedTicketLookup::Present,
            Ok(false) | Err(_) => QueueApplyObservedTicketLookup::Missing,
        }
    }

    fn calculated_base_fee_drops(&self) -> i64 {
        self.calculated_base_fee_drops
    }

    fn fee_paid_drops(&self) -> i64 {
        self.fee_paid_drops
    }

    fn default_base_fee_drops(&self) -> i64 {
        self.default_base_fee_drops
    }

    fn metrics_snapshot(&self) -> QueueFeeMetricsSnapshot {
        self.metrics_snapshot
    }

    fn open_ledger_tx_count(&self) -> usize {
        self.open_ledger.tx_count()
    }

    fn open_ledger_seq(&self) -> u32 {
        self.open_ledger.ledger_current_index
    }

    fn reserve_drops(&self) -> u64 {
        self.reserve_drops
    }

    fn base_fee_drops(&self) -> u64 {
        self.base_fee_drops
    }
}

pub type AppOpenLedger = OpenLedger<AppOpenLedgerView>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AppTxQLock;

impl QueueAcceptLockScope for AppTxQLock {}
impl QueueApplyLockScope for AppTxQLock {}

#[derive(Clone)]
pub struct SharedAppOpenLedger(Arc<AppOpenLedger>);

impl SharedAppOpenLedger {
    pub fn new(open_ledger: AppOpenLedger) -> Self {
        Self(Arc::new(open_ledger))
    }

    pub fn live_current_ledger_index(&self) -> Option<u32> {
        let current_index = self.current().ledger_current_index;
        (current_index != 0).then_some(current_index)
    }
}

impl crate::consensus::rcl_consensus::RclConsensusOpenLedgerSource for SharedAppOpenLedger {
    fn current_open_transactions(&self) -> Vec<Arc<protocol::STTx>> {
        self.current_synchronized()
            .ordered_txs()
            .into_iter()
            .map(|record| record.tx.clone())
            .collect()
    }

    fn has_open_transactions(&self) -> bool {
        self.current().tx_count() > 0
    }

    fn accept_consensus_ledger(
        &self,
        next_seq: u32,
        base_fee: u64,
        parent_hash: &basics::base_uint::Uint256,
        accepted_ids: &std::collections::HashSet<basics::base_uint::Uint256>,
    ) {
        // Capture anything left in the OLD open ledger that did NOT make it
        // into the just-accepted set, so it can be carried forward into the
        // new one -- matching the reference's `getOpenLedger().accept(...)`
        // reseeding from `localTxs_`/leftover retriable transactions
        // instead of a full destructive reset. See the trait doc comment
        // for why this matters (transactions submitted between `on_close`'s
        // capture and this reset would otherwise be silently lost).
        let leftover: Vec<std::sync::Arc<protocol::STTx>> = self
            .current_open_transactions()
            .into_iter()
            .filter(|tx| !accepted_ids.contains(&tx.get_transaction_id()))
            .collect();

        self.modify(|view| {
            *view = AppOpenLedgerView::with_parent_hash(next_seq, base_fee, *parent_hash);
            for tx in leftover {
                view.push_transaction(tx);
            }
            true
        });
    }
}

impl std::fmt::Debug for SharedAppOpenLedger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SharedAppOpenLedger")
            .field(&self.current())
            .finish()
    }
}

impl Deref for SharedAppOpenLedger {
    type Target = AppOpenLedger;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

#[derive(Clone)]
pub struct SharedAppTxQ(Arc<Mutex<AppTxQ>>);

impl SharedAppTxQ {
    pub fn new(tx_q: AppTxQ) -> Self {
        Self(Arc::new(Mutex::new(tx_q)))
    }

    pub fn set_standalone(&self, standalone: bool) {
        self.lock().set_standalone(standalone);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AppTxQ> {
        self.0.lock().expect("app txq mutex must not be poisoned")
    }

    pub fn current_max_size(&self) -> Option<usize> {
        self.lock().current_max_size()
    }

    pub fn metrics_snapshot(&self) -> QueueFeeMetricsSnapshot {
        self.lock().metrics_snapshot()
    }

    pub fn get_account_txs<Lock>(
        &self,
        lock: &mut Lock,
        account_id: &AccountID,
    ) -> Vec<tx::TxDetails<AppTxQTransaction, AppTxQAccount>>
    where
        Lock: QueueAcceptLockScope,
    {
        self.lock().get_account_txs(lock, account_id)
    }

    pub fn get_metrics<Lock, View>(&self, lock: &mut Lock, view: &View) -> tx::QueueTxQMetrics
    where
        Lock: QueueAcceptLockScope,
        View: QueueTxQMetricsView,
    {
        self.lock().get_metrics(lock, view)
    }

    pub fn get_rpc_fee_report<Lock, View>(
        &self,
        lock: &mut Lock,
        view: &View,
    ) -> tx::QueueTxQRpcReport
    where
        Lock: QueueAcceptLockScope,
        View: QueueTxQRpcView,
    {
        self.lock().get_rpc_fee_report(lock, view)
    }

    pub fn process_closed_ledger<Lock, App, View>(
        &self,
        lock: &mut Lock,
        app: &App,
        view: &View,
        time_leap: bool,
    ) -> tx::ClosedLedgerMaintenanceWithMetrics<AppTxQAccount>
    where
        Lock: QueueAcceptLockScope,
        App: tx::QueueTxQClosedLedgerAppSource<View>,
        View: tx::QueueTxQClosedLedgerView,
    {
        self.lock()
            .process_closed_ledger(lock, app, view, time_leap)
    }

    pub fn accept<Lock, App, View>(
        &self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
    ) -> tx::QueueAcceptEntryResult<AppTxQAccount>
    where
        Lock: QueueAcceptLockScope,
        App: tx::QueueAcceptLiveApplyRuntime<
                AppTxQAccount,
                AppTxQTransaction,
                AppTxQJournalTag,
                AppTxQParentBatchId,
            >,
        View: tx::QueueAcceptLedgerViewSource,
    {
        self.lock().accept(lock, app, view)
    }

    pub fn apply_with_owned_metrics_and_derived_preflight_facts_and_hold_admission<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
    ) -> tx::QueueApplyPreflightStage<
        AppTxQAccount,
        AppTxQTransaction,
        AppTxQJournalTag,
        AppTxQParentBatchId,
        TxId,
    >
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + std::fmt::Display,
        App: tx::QueueApplyAppRuntime<AppTxQTransaction, AppTxQJournalTag, AppTxQParentBatchId>,
        View: tx::QueueApplyLedgerViewSource<AppTxQAccount>,
        TxSource:
            tx::QueueApplyHoldPreflightTxSource<Account = AppTxQAccount, TransactionId = TxId>,
    {
        self.lock()
            .apply_with_owned_metrics_and_derived_preflight_facts_and_hold_admission(
                lock, app, view, tx_source,
            )
    }

    pub fn current_account_txs(
        &self,
        account_id: AccountID,
    ) -> Vec<tx::TxDetails<AppTxQTransaction, AppTxQAccount>> {
        let mut lock = AppTxQLock;
        self.get_account_txs(&mut lock, &account_id)
    }

    pub fn current_rpc_report(&self, current: &AppOpenLedgerView) -> tx::QueueTxQRpcReport {
        let mut lock = AppTxQLock;
        self.get_rpc_fee_report(&mut lock, current)
    }

    pub fn current_metrics(&self, current: &AppOpenLedgerView) -> tx::QueueTxQMetrics {
        let mut lock = AppTxQLock;
        self.get_metrics(&mut lock, current)
    }
}

impl std::fmt::Debug for SharedAppTxQ {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SharedAppTxQ")
            .field(&self.current_max_size())
            .finish()
    }
}

fn default_app_open_ledger() -> SharedAppOpenLedger {
    SharedAppOpenLedger::new(OpenLedger::new(AppOpenLedgerView::default()))
}

fn default_app_tx_q() -> SharedAppTxQ {
    let setup = TxQSetup::default();
    SharedAppTxQ::new(AppTxQ::new_from_setup(
        setup,
        None,
        QueueAcceptOwnerState::new(Uint256::from_u64(0)),
        QueueViews::new(BTreeMap::new(), Vec::new()),
    ))
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub wallet_db_path: PathBuf,
    pub path_search_old: u32,
    pub path_search: u32,
    pub path_search_fast: u32,
    pub path_search_max: u32,
    pub relay_untrusted_validations: bool,
    pub standalone: bool,
    pub start_up: xrpl_core::StartUpType,
    pub start_ledger: Option<String>,
    pub do_import: bool,
    pub validation_quorum: usize,
    pub validation_seed: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            wallet_db_path: PathBuf::new(),
            path_search_old: 2,
            path_search: 2,
            path_search_fast: 2,
            path_search_max: 2,
            relay_untrusted_validations: false,
            standalone: false,
            start_up: xrpl_core::StartUpType::Fresh,
            start_ledger: None,
            do_import: false,
            validation_quorum: 1,
            validation_seed: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppJournalEntry {
    pub owner: String,
    pub level: String,
    pub message: String,
}

#[derive(Debug)]
pub struct AppJournal {
    owner: String,
    entries: Mutex<Vec<AppJournalEntry>>,
}

impl AppJournal {
    pub fn new(owner: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            entries: Mutex::new(Vec::new()),
        }
    }

    pub fn entries(&self) -> Vec<AppJournalEntry> {
        self.entries
            .lock()
            .expect("app journal mutex must not be poisoned")
            .clone()
    }

    fn record(&self, level: impl Into<String>, message: impl Into<String>) {
        self.entries
            .lock()
            .expect("app journal mutex must not be poisoned")
            .push(AppJournalEntry {
                owner: self.owner.clone(),
                level: level.into(),
                message: message.into(),
            });
    }
}

impl LoadManagerJournal for AppJournal {
    fn debug(&self, message: &str) {
        self.record("debug", message);
    }

    fn info(&self, message: &str) {
        self.record("info", message);
    }

    fn warn(&self, message: &str) {
        self.record("warn", message);
    }

    fn fatal(&self, message: &str) {
        self.record("fatal", message);
    }
}

impl BuildLedgerJournal for AppJournal {
    fn debug(&self, message: &str) {
        self.record("debug", message);
    }

    fn warn(&self, message: &str) {
        self.record("warn", message);
    }
}

impl RclValidationJournal for AppJournal {
    fn trace(&self, message: &str) {
        self.record("trace", message);
    }

    fn info(&self, message: &str) {
        self.record("info", message);
    }

    fn error(&self, message: &str) {
        self.record("error", message);
    }

    fn warn(&self, message: &str) {
        self.record("warn", message);
    }
}

impl NodeStoreJournal for AppJournal {
    fn log(&self, level: NodeStoreJournalLevel, message: &str) {
        self.record(level.to_string(), message);
    }
}

impl PerfLogJournal for AppJournal {
    fn log(&self, level: PerfLogJournalLevel, message: &str) {
        self.record(level.to_string(), message);
    }
}

impl LoadMonitorJournal for AppJournal {
    fn debug(&self, message: &str) {
        self.record("debug", message);
    }

    fn info(&self, message: &str) {
        self.record("info", message);
    }

    fn warn(&self, message: &str) {
        self.record("warn", message);
    }
}

impl PeerReservationJournal for AppJournal {
    fn warn(&self, message: &str) {
        self.record("warn", message);
    }
}

impl resource::ResourceJournal for AppJournal {
    fn log(&self, level: resource::JournalLevel, message: &str) {
        let level = match level {
            resource::JournalLevel::Trace => "trace",
            resource::JournalLevel::Debug => "debug",
            resource::JournalLevel::Info => "info",
            resource::JournalLevel::Warn => "warn",
            resource::JournalLevel::Fatal => "fatal",
        };
        self.record(level, message);
    }
}

impl ledger::LedgerCleanerJournal for AppJournal {
    fn debug(&self, message: &str) {
        self.record("debug", message);
    }

    fn info(&self, message: &str) {
        self.record("info", message);
    }
}

#[derive(Debug, Clone)]
pub struct AppLoadMonitorJournalFactory {
    logs: Arc<AppLogs>,
}

impl AppLoadMonitorJournalFactory {
    pub fn new(logs: Arc<AppLogs>) -> Self {
        Self { logs }
    }
}

impl LoadMonitorJournalFactory for AppLoadMonitorJournalFactory {
    fn make_load_monitor_journal(&self, name: &str) -> Arc<dyn LoadMonitorJournal> {
        self.logs.journal(name)
    }
}

#[derive(Debug, Default)]
pub struct AppLogs {
    journals: Mutex<HashMap<String, Arc<AppJournal>>>,
}

impl AppLogs {
    pub fn journal(&self, name: &str) -> Arc<AppJournal> {
        let mut journals = self
            .journals
            .lock()
            .expect("app logs mutex must not be poisoned");
        journals
            .entry(name.to_owned())
            .or_insert_with(|| Arc::new(AppJournal::new(name)))
            .clone()
    }
}

#[derive(Clone)]
pub struct ApplicationRegistryOwners {
    pub temp_node_cache: Arc<TreeNodeCache>,
    pub cached_sles: Arc<CachedSles>,
    pub network_id_service: FixedNetworkIdService,
    pub hash_router: Arc<HashRouter>,
    pub validator_sites: Arc<ValidatorSite>,
    pub manifest_cache: Arc<ManifestCache>,
    pub cluster: Arc<Cluster>,
    pub resource_manager: Arc<ResourceManager>,
    pub inbound_ledgers: AppInboundLedgers,
    pub inbound_transactions: AppInboundTransactions,
    pub accepted_ledger_cache: AppAcceptedLedgerCache,
    pub ledger_cleaner: Arc<LedgerCleaner>,
    pub ledger_replayer: Arc<Mutex<LedgerReplayer>>,
    pub pending_saves: Arc<PendingSaves>,
    pub open_ledger: SharedAppOpenLedger,
    pub order_book_db: Arc<OrderBookDB>,
    pub path_request_manager: Arc<crate::paths::PathRequestManager>,
    pub server_handler: Arc<AppServerHandler>,
    pub tx_q: SharedAppTxQ,
    pub logs: Arc<AppLogs>,
    pub load_monitor_journal_factory: Arc<AppLoadMonitorJournalFactory>,
    pub wallet_db: Arc<DatabaseCon>,
    pub peer_reservations: Arc<PeerReservationTable<PublicKey>>,
    pub network_ops_state_sink:
        Arc<Mutex<Option<Arc<crate::network::network_ops::SharedNetworkOpsState>>>>,
    pub perf_log: Option<Arc<PerfLogImp>>,
    pub node_store: Option<SHAMapStoreNodeStore>,
    pub relational_database: Option<Arc<SqliteSHAMapStoreRelational>>,
    pub ledger_db: Option<Arc<rdb::LedgerDb>>,
    pub trap_tx_id: Option<Uint256>,
    pub config: AppConfig,
}

impl std::fmt::Debug for ApplicationRegistryOwners {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApplicationRegistryOwners")
            .field("wallet_db_path", &self.config.wallet_db_path)
            .field("has_perf_log", &self.perf_log.is_some())
            .field("has_node_store", &self.node_store.is_some())
            .field(
                "has_relational_database",
                &self.relational_database.is_some(),
            )
            .field(
                "peer_reservation_count",
                &self.peer_reservations.list().len(),
            )
            .field(
                "load_monitor_journal_factory",
                &"AppLoadMonitorJournalFactory",
            )
            .finish()
    }
}

impl ApplicationRegistryOwners {
    pub fn new() -> Result<Self, String> {
        let wallet_db_path = unique_wallet_db_dir();
        fs::create_dir_all(&wallet_db_path).map_err(|error| error.to_string())?;
        let wallet_db = Arc::new(DatabaseCon::new_at_path(
            &wallet_db_path,
            WALLET_DB_NAME,
            &[],
            WALLET_DB_INIT,
        )?);
        let logs = Arc::new(AppLogs::default());
        let load_monitor_journal_factory =
            Arc::new(AppLoadMonitorJournalFactory::new(Arc::clone(&logs)));
        let peer_reservations = Arc::new(PeerReservationTable::new_with_journal(
            logs.journal("peer_reservations"),
        ));
        peer_reservations.load_from_database(Arc::clone(&wallet_db))?;

        let network_ops_state_sink = Arc::new(Mutex::new(None));
        let perf_log = Arc::new(PerfLogImp::new_with_hostname(
            PerfLogSetup::default(),
            Vec::new(),
            Arc::new(StateAccountingReportSource {
                network_ops_state: Arc::clone(&network_ops_state_sink),
            }),
            logs.journal("perf_log"),
            Arc::new(|| {}),
            "application-root",
        ));
        let inbound_ledgers = Arc::new(Mutex::new(InboundLedgersLocal::new()));
        let inbound_transactions = Arc::new(Mutex::new(InboundTransactions::new(Arc::new(
            SimplePeerSetBuilder::new(Vec::new()),
        ))));
        let accepted_ledger_cache = Arc::new(TaggedCache::new(
            "AcceptedLedger",
            4,
            TimeDuration::minutes(1),
            MonotonicClock::default(),
        ));

        let temp_node_cache = Arc::new(TreeNodeCache::new(
            "TreeNodeCache",
            32768,
            TimeDuration::minutes(1),
            MonotonicClock::default(),
        ));

        let cached_sles = Arc::new(CachedSles::new(
            "CachedSLEs",
            16384,
            TimeDuration::minutes(1),
            MonotonicClock::default(),
        ));

        let resource_manager = Arc::new(resource::make_manager(
            Arc::new(resource::NullCollector),
            logs.journal("resource"),
        ));

        let ledger_cleaner = Arc::new(LedgerCleaner::new(
            Arc::new(ledger::NullLedgerCleanerRangeProvider),
            Arc::new(ledger::NullLedgerCleanerRuntime),
            logs.journal("ledger_cleaner"),
        ));

        let ledger_replayer = Arc::new(Mutex::new(LedgerReplayer::new(Arc::new(
            SimplePeerSetBuilder::new(Vec::new()),
        ))));

        let pending_saves = Arc::new(PendingSaves::new());

        let order_book_db = Arc::new(OrderBookDB::new(ledger::OrderBookDBConfig::default()));

        let path_request_manager = Arc::new(crate::paths::PathRequestManager::new());

        Ok(Self {
            temp_node_cache,
            cached_sles,
            network_id_service: FixedNetworkIdService::new(0),
            hash_router: Arc::new(HashRouter::new(HashRouterSetup::default())),
            validator_sites: Arc::new(ValidatorSite::new(Duration::from_secs(30))),
            manifest_cache: Arc::new(ManifestCache::new()),
            cluster: Arc::new(Cluster::new()),
            resource_manager,
            inbound_ledgers,
            inbound_transactions,
            accepted_ledger_cache,
            ledger_cleaner,
            ledger_replayer,
            pending_saves,
            open_ledger: default_app_open_ledger(),
            order_book_db,
            path_request_manager,
            server_handler: Arc::new(AppServerHandler::default()),
            tx_q: default_app_tx_q(),
            logs,
            load_monitor_journal_factory,
            wallet_db,
            peer_reservations,
            network_ops_state_sink,
            perf_log: Some(perf_log),
            node_store: None,
            relational_database: None,
            ledger_db: None,
            trap_tx_id: None,
            config: AppConfig {
                wallet_db_path,
                path_search_old: 2,
                path_search: 2,
                path_search_fast: 2,
                path_search_max: 3,
                relay_untrusted_validations: false,
                standalone: false,
                start_up: xrpl_core::StartUpType::Fresh,
                start_ledger: None,
                do_import: false,
                validation_quorum: 1,
                validation_seed: None,
            },
        })
    }

    pub fn attach_perf_log(&mut self, perf_log: Arc<PerfLogImp>) -> Option<Arc<PerfLogImp>> {
        self.perf_log.replace(perf_log)
    }

    pub fn load_monitor_journal_factory(&self) -> Arc<dyn LoadMonitorJournalFactory> {
        self.load_monitor_journal_factory.clone()
    }
}

fn unique_wallet_db_dir() -> PathBuf {
    let sequence = WALLET_DB_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "xrpld-application-root-wallet-{}-{sequence}",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        APP_OPEN_LEDGER_DEFAULT_BASE_FEE_DROPS, AppLogs, AppOpenLedgerTxRecord, AppPlaceholder,
        unique_wallet_db_dir,
    };
    use crate::load::load_manager::LoadManagerJournal;
    use basics::base_uint::Uint256;
    use protocol::JsonValue;
    use std::sync::Arc;
    use xrpl_core::{HashRouterFlags, LoadMonitorJournalFactory, NetworkIDService};

    #[test]
    fn app_logs_reuse_named_journals_and_keep_entries() {
        let logs = AppLogs::default();
        let alpha = logs.journal("alpha");
        let beta = logs.journal("beta");

        alpha.info("hello");
        beta.warn("world");

        assert_eq!(alpha.entries().len(), 1);
        assert_eq!(beta.entries().len(), 1);
        assert_eq!(alpha.entries()[0].message, "hello");
        assert_eq!(beta.entries()[0].level, "warn");
        assert!(!std::ptr::eq(Arc::as_ptr(&alpha), Arc::as_ptr(&beta),));
    }

    #[test]
    fn app_load_monitor_journal_factory_reuses_named_journals() {
        let logs = Arc::new(AppLogs::default());
        let factory = super::AppLoadMonitorJournalFactory::new(Arc::clone(&logs));
        let first = factory.make_load_monitor_journal("job-a");
        let second = factory.make_load_monitor_journal("job-a");
        first.debug("hello");
        second.warn("world");

        let journal = logs.journal("job-a");
        assert_eq!(journal.entries().len(), 2);
        assert_eq!(journal.entries()[0].message, "hello");
        assert_eq!(journal.entries()[1].message, "world");
    }

    #[test]
    fn application_registry_owners_seed_real_default_runtime_shell_state() {
        let owners = super::ApplicationRegistryOwners::new().expect("registry owners");

        assert_eq!(owners.network_id_service.get_network_id(), 0);
        assert_eq!(
            owners.hash_router.get_flags(Uint256::default()),
            HashRouterFlags::UNDEFINED
        );
        assert_eq!(owners.manifest_cache.sequence(), 0);
        assert!(matches!(
            owners.validator_sites.get_json(),
            JsonValue::Object(json) if json.contains_key("validator_sites")
        ));

        let mut cluster_count = 0usize;
        owners.cluster.for_each(|_| cluster_count += 1);
        assert_eq!(cluster_count, 0);
        assert!(owners.open_ledger.empty::<AppOpenLedgerTxRecord>());
        assert_eq!(owners.open_ledger.current().ledger_current_index, 0);
        assert_eq!(
            owners.open_ledger.current().base_fee_drops,
            APP_OPEN_LEDGER_DEFAULT_BASE_FEE_DROPS
        );
        assert_eq!(owners.tx_q.current_max_size(), None);
    }

    #[test]
    fn wallet_db_dir_is_unique_per_call() {
        let first = unique_wallet_db_dir();
        let second = unique_wallet_db_dir();
        assert_ne!(first, second);
        assert!(first.starts_with(std::env::temp_dir()));
    }

    #[test]
    fn placeholder_is_a_zero_sized_marker() {
        assert_eq!(std::mem::size_of::<AppPlaceholder>(), 0);
    }
}
