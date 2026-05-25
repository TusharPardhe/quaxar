//! Shared view seam for app-backed `server_info` / `server_state`.
//!
//! The helper stack is intentionally reusable for both a borrowed
//! `ApplicationRoot` and an owned source that clones the shared app state
//! handles needed by a server runtime component.

use std::sync::Arc;
use std::time::Duration;

use app::{
    AmendmentStatus, AppOpenLedgerView, ApplicationRoot, JobQueue, NetworkOpsCurrentLedgerState,
    OverlayStatusSource, PublishedServerPortsSource, SharedAppOpenLedger, SharedAppTxQ,
    SharedLedgerMasterState, SharedLoadFeeTrack, SharedNetworkOpsState, SharedTransaction,
    StatusMetricsSource, StatusRpcSnapshot, StatusRpcState, SystemTimeKeeperClock, TimeKeeper,
    TransactionMaster, UnsupportedMajorityWarningDetails, ValidatorList,
    ValidatorListStatusSnapshot,
};
use basics::base_uint::Uint256;
use ledger::{Ledger, TxsRawView};
use protocol::{PublicKey, SecretKey};
use time::OffsetDateTime;
use tx::QueueTxQRpcReport;

pub trait AppServerInfoView {
    fn app(&self) -> Option<&ApplicationRoot> {
        None
    }

    fn status_snapshot(&self) -> StatusRpcSnapshot;
    fn status_metrics(&self) -> Option<Arc<dyn StatusMetricsSource>>;
    fn overlay_status(&self) -> Option<Arc<dyn OverlayStatusSource>>;
    fn published_server_ports(&self) -> Option<Arc<dyn PublishedServerPortsSource>>;
    fn validators(&self) -> Arc<ValidatorList>;
    fn validator_list_status_snapshot(&self) -> ValidatorListStatusSnapshot;
    fn admin_pubkey_validator(&self) -> String;
    fn current_server_time_string(&self) -> String;
    fn current_close_time_seconds(&self) -> u32;
    fn close_time_offset_seconds(&self) -> i64;
    fn validated_ledger_age(&self) -> Duration;
    fn validated_ledger(&self) -> Option<Arc<Ledger>>;
    fn closed_ledger(&self) -> Option<Arc<Ledger>>;
    fn published_ledger(&self) -> Option<Arc<Ledger>>;
    fn load_fee_track(&self) -> Arc<SharedLoadFeeTrack>;
    fn job_queue(&self) -> JobQueue;
    fn network_ops_runtime(&self) -> Option<Arc<app::AppNetworkOpsRuntime>> {
        None
    }
    fn network_ops_operating_mode_string(&self) -> &'static str;
    fn need_network_ledger(&self) -> bool;
    fn amendment_blocked(&self) -> bool;
    fn unl_blocked(&self) -> bool;
    fn unsupported_majority_warned(&self) -> bool;
    fn unsupported_majority_warning_details(&self) -> Option<UnsupportedMajorityWarningDetails>;
    fn amendment_status(&self) -> Arc<AmendmentStatus>;
    fn node_identity(&self) -> Option<(PublicKey, SecretKey)>;
    fn validation_public_key(&self) -> Option<PublicKey>;
    fn status_rpc_current_ledger_index(&self) -> Option<u32>;
    fn status_rpc_queue_report(&self) -> Option<QueueTxQRpcReport>;
    fn path_search_old(&self) -> u32;
    fn path_search(&self) -> u32;
    fn path_search_fast(&self) -> u32;
    fn path_search_max(&self) -> u32;
    fn standalone(&self) -> bool;
    fn network_id(&self) -> u32;
    fn fetch_cached_transaction(&self, txn_id: &Uint256) -> Option<SharedTransaction>;
    fn txn_id_from_index(&self, _ledger_seq: u32, _txn_index: u32) -> Option<Uint256> {
        None
    }
    fn accept_standalone_ledger(&self) -> Result<u32, String> {
        Err("ledger_accept is unavailable for this runtime source".to_owned())
    }
}

fn merge_live_status_fallbacks(
    mut snapshot: StatusRpcSnapshot,
    current_ledger_index: Option<u32>,
    queue_report: Option<QueueTxQRpcReport>,
) -> StatusRpcSnapshot {
    if snapshot.current_ledger_index.is_none() {
        snapshot.current_ledger_index = current_ledger_index;
    }
    if snapshot.queue_report.is_none() {
        snapshot.queue_report = queue_report;
    }
    snapshot
}

fn live_queue_report(
    open_ledger: &SharedAppOpenLedger,
    tx_q: &SharedAppTxQ,
) -> tx::QueueTxQRpcReport {
    let current = open_ledger.current();
    tx_q.current_rpc_report(current.as_ref())
}

impl<T: AppServerInfoView + ?Sized> AppServerInfoView for &T {
    fn app(&self) -> Option<&ApplicationRoot> {
        (**self).app()
    }

    fn status_snapshot(&self) -> StatusRpcSnapshot {
        (**self).status_snapshot()
    }

    fn status_metrics(&self) -> Option<Arc<dyn StatusMetricsSource>> {
        (**self).status_metrics()
    }

    fn overlay_status(&self) -> Option<Arc<dyn OverlayStatusSource>> {
        (**self).overlay_status()
    }

    fn published_server_ports(&self) -> Option<Arc<dyn PublishedServerPortsSource>> {
        (**self).published_server_ports()
    }

    fn validators(&self) -> Arc<ValidatorList> {
        (**self).validators()
    }

    fn validator_list_status_snapshot(&self) -> ValidatorListStatusSnapshot {
        (**self).validator_list_status_snapshot()
    }

    fn admin_pubkey_validator(&self) -> String {
        (**self).admin_pubkey_validator()
    }

    fn current_server_time_string(&self) -> String {
        (**self).current_server_time_string()
    }

    fn current_close_time_seconds(&self) -> u32 {
        (**self).current_close_time_seconds()
    }

    fn close_time_offset_seconds(&self) -> i64 {
        (**self).close_time_offset_seconds()
    }

    fn validated_ledger_age(&self) -> Duration {
        (**self).validated_ledger_age()
    }

    fn validated_ledger(&self) -> Option<Arc<Ledger>> {
        (**self).validated_ledger()
    }

    fn closed_ledger(&self) -> Option<Arc<Ledger>> {
        (**self).closed_ledger()
    }

    fn published_ledger(&self) -> Option<Arc<Ledger>> {
        (**self).published_ledger()
    }

    fn load_fee_track(&self) -> Arc<SharedLoadFeeTrack> {
        (**self).load_fee_track()
    }

    fn job_queue(&self) -> JobQueue {
        (**self).job_queue()
    }

    fn network_ops_runtime(&self) -> Option<Arc<app::AppNetworkOpsRuntime>> {
        (**self).network_ops_runtime()
    }

    fn network_ops_operating_mode_string(&self) -> &'static str {
        (**self).network_ops_operating_mode_string()
    }

    fn need_network_ledger(&self) -> bool {
        (**self).need_network_ledger()
    }

    fn amendment_blocked(&self) -> bool {
        (**self).amendment_blocked()
    }

    fn unl_blocked(&self) -> bool {
        (**self).unl_blocked()
    }

    fn unsupported_majority_warned(&self) -> bool {
        (**self).unsupported_majority_warned()
    }

    fn unsupported_majority_warning_details(&self) -> Option<UnsupportedMajorityWarningDetails> {
        (**self).unsupported_majority_warning_details()
    }

    fn amendment_status(&self) -> Arc<AmendmentStatus> {
        (**self).amendment_status()
    }

    fn node_identity(&self) -> Option<(PublicKey, SecretKey)> {
        (**self).node_identity()
    }

    fn validation_public_key(&self) -> Option<PublicKey> {
        (**self).validation_public_key()
    }

    fn status_rpc_current_ledger_index(&self) -> Option<u32> {
        (**self).status_rpc_current_ledger_index()
    }

    fn status_rpc_queue_report(&self) -> Option<QueueTxQRpcReport> {
        (**self).status_rpc_queue_report()
    }

    fn path_search_old(&self) -> u32 {
        (**self).path_search_old()
    }

    fn path_search(&self) -> u32 {
        (**self).path_search()
    }

    fn path_search_fast(&self) -> u32 {
        (**self).path_search_fast()
    }

    fn path_search_max(&self) -> u32 {
        (**self).path_search_max()
    }

    fn standalone(&self) -> bool {
        (**self).standalone()
    }

    fn network_id(&self) -> u32 {
        (**self).network_id()
    }

    fn fetch_cached_transaction(&self, txn_id: &Uint256) -> Option<SharedTransaction> {
        (**self).fetch_cached_transaction(txn_id)
    }

    fn accept_standalone_ledger(&self) -> Result<u32, String> {
        (**self).accept_standalone_ledger()
    }
}

#[derive(Clone)]
pub struct OwnedApplicationServerInfo {
    app: ApplicationRoot,
    status_rpc_state: Arc<StatusRpcState>,
    open_ledger: SharedAppOpenLedger,
    tx_q: SharedAppTxQ,
    status_metrics: Option<Arc<dyn StatusMetricsSource>>,
    overlay_status: Option<Arc<dyn OverlayStatusSource>>,
    published_server_ports: Option<Arc<dyn PublishedServerPortsSource>>,
    validators: Arc<ValidatorList>,
    load_fee_track: Arc<SharedLoadFeeTrack>,
    job_queue: JobQueue,
    network_ops_runtime: Option<Arc<app::AppNetworkOpsRuntime>>,
    network_ops_state: Arc<SharedNetworkOpsState>,
    ledger_master_state: Arc<SharedLedgerMasterState>,
    amendment_status: Arc<AmendmentStatus>,
    time_keeper: Arc<TimeKeeper<SystemTimeKeeperClock>>,
    transaction_master: Arc<TransactionMaster>,
    node_identity: Option<(PublicKey, SecretKey)>,
    validation_public_key: Option<PublicKey>,
    path_search_old: u32,
    path_search: u32,
    path_search_fast: u32,
    path_search_max: u32,
    standalone: bool,
    network_id: u32,
}

impl OwnedApplicationServerInfo {
    pub fn from_application_root(app: &ApplicationRoot) -> Self {
        Self {
            app: app.clone(),
            status_rpc_state: app.status_rpc_state(),
            open_ledger: app.open_ledger().clone(),
            tx_q: app.tx_q().clone(),
            status_metrics: app.status_metrics(),
            overlay_status: app.overlay_status(),
            published_server_ports: app.published_server_ports(),
            validators: app.validators(),
            load_fee_track: app.load_fee_track(),
            job_queue: app.job_queue().clone(),
            network_ops_runtime: app.network_ops_runtime(),
            network_ops_state: app.network_ops_state(),
            ledger_master_state: app.ledger_master_state(),
            amendment_status: app.amendment_status(),
            time_keeper: app.shared_time_keeper(),
            transaction_master: app.transaction_master(),
            node_identity: app.node_identity(),
            validation_public_key: app.validation_public_key(),
            path_search_old: app.path_search_old(),
            path_search: app.path_search(),
            path_search_fast: app.path_search_fast(),
            path_search_max: app.path_search_max(),
            standalone: app.standalone(),
            network_id: app.network_id(),
        }
    }
}

impl From<&ApplicationRoot> for OwnedApplicationServerInfo {
    fn from(app: &ApplicationRoot) -> Self {
        Self::from_application_root(app)
    }
}

impl AppServerInfoView for ApplicationRoot {
    fn app(&self) -> Option<&ApplicationRoot> {
        Some(self)
    }

    fn status_snapshot(&self) -> StatusRpcSnapshot {
        merge_live_status_fallbacks(
            self.status_rpc_state().snapshot(),
            self.live_current_ledger_index(),
            Some(self.tx_q_rpc_report()),
        )
    }

    fn status_metrics(&self) -> Option<Arc<dyn StatusMetricsSource>> {
        self.status_metrics()
    }

    fn overlay_status(&self) -> Option<Arc<dyn OverlayStatusSource>> {
        self.overlay_status()
    }

    fn published_server_ports(&self) -> Option<Arc<dyn PublishedServerPortsSource>> {
        self.published_server_ports()
    }

    fn validators(&self) -> Arc<ValidatorList> {
        self.validators()
    }

    fn validator_list_status_snapshot(&self) -> ValidatorListStatusSnapshot {
        self.validator_list_status_snapshot()
    }

    fn admin_pubkey_validator(&self) -> String {
        self.admin_pubkey_validator()
    }

    fn current_server_time_string(&self) -> String {
        self.current_server_time_string()
    }

    fn current_close_time_seconds(&self) -> u32 {
        self.current_close_time_seconds()
    }

    fn close_time_offset_seconds(&self) -> i64 {
        self.close_time_offset_seconds()
    }

    fn validated_ledger_age(&self) -> Duration {
        self.validated_ledger_age()
    }

    fn validated_ledger(&self) -> Option<Arc<Ledger>> {
        self.validated_ledger()
    }

    fn closed_ledger(&self) -> Option<Arc<Ledger>> {
        self.closed_ledger()
    }

    fn published_ledger(&self) -> Option<Arc<Ledger>> {
        self.published_ledger()
    }

    fn load_fee_track(&self) -> Arc<SharedLoadFeeTrack> {
        self.load_fee_track()
    }

    fn job_queue(&self) -> JobQueue {
        self.job_queue().clone()
    }

    fn network_ops_runtime(&self) -> Option<Arc<app::AppNetworkOpsRuntime>> {
        self.network_ops_runtime()
    }

    fn network_ops_operating_mode_string(&self) -> &'static str {
        self.network_ops_operating_mode_string()
    }

    fn need_network_ledger(&self) -> bool {
        self.need_network_ledger()
    }

    fn amendment_blocked(&self) -> bool {
        self.amendment_blocked()
    }

    fn unl_blocked(&self) -> bool {
        self.unl_blocked()
    }

    fn unsupported_majority_warned(&self) -> bool {
        self.unsupported_majority_warned()
    }

    fn unsupported_majority_warning_details(&self) -> Option<UnsupportedMajorityWarningDetails> {
        self.unsupported_majority_warning_details()
    }

    fn amendment_status(&self) -> Arc<AmendmentStatus> {
        self.amendment_status()
    }

    fn node_identity(&self) -> Option<(PublicKey, SecretKey)> {
        self.node_identity()
    }

    fn validation_public_key(&self) -> Option<PublicKey> {
        self.validation_public_key()
    }

    fn status_rpc_current_ledger_index(&self) -> Option<u32> {
        ApplicationRoot::status_rpc_current_ledger_index(self)
            .or_else(|| ApplicationRoot::live_current_ledger_index(self))
    }

    fn status_rpc_queue_report(&self) -> Option<QueueTxQRpcReport> {
        ApplicationRoot::status_rpc_queue_report(self)
            .or_else(|| Some(ApplicationRoot::tx_q_rpc_report(self)))
    }

    fn path_search_old(&self) -> u32 {
        self.path_search_old()
    }

    fn path_search(&self) -> u32 {
        self.path_search()
    }

    fn path_search_fast(&self) -> u32 {
        self.path_search_fast()
    }

    fn path_search_max(&self) -> u32 {
        self.path_search_max()
    }

    fn standalone(&self) -> bool {
        ApplicationRoot::standalone(self)
    }

    fn network_id(&self) -> u32 {
        ApplicationRoot::network_id(self)
    }

    fn fetch_cached_transaction(&self, txn_id: &Uint256) -> Option<SharedTransaction> {
        ApplicationRoot::fetch_cached_transaction(self, txn_id)
    }

    fn txn_id_from_index(&self, ledger_seq: u32, txn_index: u32) -> Option<Uint256> {
        self.ledger_master_runtime()
            .and_then(|lm| lm.ledger_master().txn_id_from_index(ledger_seq, txn_index))
    }

    fn accept_standalone_ledger(&self) -> Result<u32, String> {
        ApplicationRoot::accept_standalone_ledger(self)
    }
}

impl AppServerInfoView for OwnedApplicationServerInfo {
    fn app(&self) -> Option<&ApplicationRoot> {
        Some(&self.app)
    }

    fn status_snapshot(&self) -> StatusRpcSnapshot {
        merge_live_status_fallbacks(
            self.status_rpc_state.snapshot(),
            self.open_ledger.live_current_ledger_index(),
            Some(live_queue_report(&self.open_ledger, &self.tx_q)),
        )
    }

    fn status_metrics(&self) -> Option<Arc<dyn StatusMetricsSource>> {
        self.status_metrics.as_ref().map(Arc::clone)
    }

    fn overlay_status(&self) -> Option<Arc<dyn OverlayStatusSource>> {
        self.overlay_status.as_ref().map(Arc::clone)
    }

    fn published_server_ports(&self) -> Option<Arc<dyn PublishedServerPortsSource>> {
        self.published_server_ports.as_ref().map(Arc::clone)
    }

    fn validators(&self) -> Arc<ValidatorList> {
        Arc::clone(&self.validators)
    }

    fn validator_list_status_snapshot(&self) -> ValidatorListStatusSnapshot {
        self.validators.status_snapshot()
    }

    fn admin_pubkey_validator(&self) -> String {
        self.validation_public_key
            .and(self.validators.local_public_key())
            .map(|public_key| public_key.to_node_public_base58())
            .unwrap_or_else(|| "none".to_owned())
    }

    fn current_server_time_string(&self) -> String {
        basics::chrono::to_string(OffsetDateTime::now_utc())
    }

    fn current_close_time_seconds(&self) -> u32 {
        self.time_keeper.close_time().as_seconds()
    }

    fn close_time_offset_seconds(&self) -> i64 {
        self.time_keeper.close_offset().whole_seconds()
    }

    fn validated_ledger_age(&self) -> Duration {
        self.ledger_master_state.validated_ledger_age()
    }

    fn validated_ledger(&self) -> Option<Arc<Ledger>> {
        self.ledger_master_state.validated_ledger()
    }

    fn closed_ledger(&self) -> Option<Arc<Ledger>> {
        self.ledger_master_state.closed_ledger()
    }

    fn published_ledger(&self) -> Option<Arc<Ledger>> {
        self.ledger_master_state.published_ledger()
    }

    fn load_fee_track(&self) -> Arc<SharedLoadFeeTrack> {
        Arc::clone(&self.load_fee_track)
    }

    fn job_queue(&self) -> JobQueue {
        self.job_queue.clone()
    }

    fn network_ops_runtime(&self) -> Option<Arc<app::AppNetworkOpsRuntime>> {
        self.network_ops_runtime.clone()
    }

    fn network_ops_operating_mode_string(&self) -> &'static str {
        self.network_ops_state.str_operating_mode()
    }

    fn need_network_ledger(&self) -> bool {
        self.network_ops_state.need_network_ledger()
    }

    fn amendment_blocked(&self) -> bool {
        self.network_ops_state.amendment_blocked()
    }

    fn unl_blocked(&self) -> bool {
        self.network_ops_state.unl_blocked()
    }

    fn unsupported_majority_warned(&self) -> bool {
        self.amendment_status.unsupported_majority_warned()
    }

    fn unsupported_majority_warning_details(&self) -> Option<UnsupportedMajorityWarningDetails> {
        self.amendment_status.unsupported_majority_warning_details()
    }

    fn amendment_status(&self) -> Arc<AmendmentStatus> {
        Arc::clone(&self.amendment_status)
    }

    fn node_identity(&self) -> Option<(PublicKey, SecretKey)> {
        self.node_identity.clone()
    }

    fn validation_public_key(&self) -> Option<PublicKey> {
        self.validation_public_key
    }

    fn status_rpc_current_ledger_index(&self) -> Option<u32> {
        self.status_rpc_state
            .current_ledger_index()
            .or_else(|| self.open_ledger.live_current_ledger_index())
    }

    fn status_rpc_queue_report(&self) -> Option<QueueTxQRpcReport> {
        self.status_rpc_state
            .queue_report()
            .or_else(|| Some(live_queue_report(&self.open_ledger, &self.tx_q)))
    }

    fn path_search_old(&self) -> u32 {
        self.path_search_old
    }

    fn path_search(&self) -> u32 {
        self.path_search
    }

    fn path_search_fast(&self) -> u32 {
        self.path_search_fast
    }

    fn path_search_max(&self) -> u32 {
        self.path_search_max
    }

    fn standalone(&self) -> bool {
        self.standalone
    }

    fn network_id(&self) -> u32 {
        self.network_id
    }

    fn fetch_cached_transaction(&self, txn_id: &Uint256) -> Option<SharedTransaction> {
        self.transaction_master.fetch_from_cache(txn_id)
    }

    fn accept_standalone_ledger(&self) -> Result<u32, String> {
        if !self.standalone {
            return Err("ledger_accept requires standalone mode".to_owned());
        }

        let current = self.open_ledger.current();
        let closed_seq = current
            .ledger_current_index
            .max(1)
            .saturating_sub(1)
            .max(
                self.ledger_master_state
                    .closed_ledger()
                    .map(|ledger| ledger.header().seq)
                    .unwrap_or(0),
            )
            .max(
                self.ledger_master_state
                    .validated_ledger()
                    .map(|ledger| ledger.header().seq)
                    .unwrap_or(0),
            )
            .max(1);
        let close_time = self.current_close_time_seconds();

        let next_open_parent_hash = self
            .ledger_master_state
            .closed_ledger()
            .or_else(|| self.ledger_master_state.validated_ledger())
            .map(|ledger| ledger.header().hash)
            .unwrap_or_default();

        if let Some(runtime) = self.network_ops_runtime.as_ref() {
            let _ = runtime.apply_held_transactions_to_queue(next_open_parent_hash, |_sync| {});
        }

        let mut closed = Ledger::from_ledger_seq_and_close_time(closed_seq, close_time, false);

        if let Some(runtime) = self.network_ops_runtime.as_ref()
            && let Some(report) = runtime.apply_pending_with(
                closed_seq,
                self.ledger_master_state
                    .validated_ledger()
                    .map(|ledger| ledger.header().seq),
                |_tx, _flags| tx::ApplyResult::new(protocol::Ter::TES_SUCCESS, true, false),
                || {},
                |_tx, _result| {},
                |_tx| {},
                |_tx| false,
                |_tx| None::<()>,
                |_tx, _deferred, _skip| {},
                |_tx| NetworkOpsCurrentLedgerState {
                    fee: protocol::XRPAmount::from_drops(current.base_fee_drops as i64),
                    account_seq: 0_u32,
                    available_seq: 0_u32,
                },
            )
        {
            for (index, entry) in report.entries.iter().enumerate() {
                if !entry.applied {
                    continue;
                }

                let _ = self.transaction_master.in_ledger(
                    entry.transaction_id,
                    closed_seq,
                    Some(index as u32),
                    Some(self.network_id),
                );

                let Some(shared_tx) = self
                    .transaction_master
                    .fetch_from_cache(&entry.transaction_id)
                else {
                    continue;
                };
                let Ok(tx) = shared_tx.lock() else {
                    continue;
                };

                let mut meta = protocol::TxMeta::new(entry.transaction_id, closed_seq);
                let mut serializer = protocol::Serializer::default();
                meta.add_raw(&mut serializer, entry.result, index as u32);

                let _ = closed.raw_tx_insert(
                    entry.transaction_id,
                    std::sync::Arc::new(protocol::Serializer::from_bytes(
                        tx.get_s_transaction().get_serializer().data(),
                    )),
                    Some(std::sync::Arc::new(serializer)),
                );
            }
        }

        closed.set_accepted(close_time, 0, true);
        let closed = std::sync::Arc::new(closed);
        self.ledger_master_state
            .note_closed_ledger(std::sync::Arc::clone(&closed));
        self.ledger_master_state
            .note_published_ledger(std::sync::Arc::clone(&closed));
        self.ledger_master_state.note_validated_ledger(closed);

        let next_open_index = closed_seq.saturating_add(1);
        let next_open = AppOpenLedgerView::new(next_open_index, current.base_fee_drops);
        let _ = self.open_ledger.modify(|view| {
            *view = next_open;
            true
        });
        self.status_rpc_state
            .set_current_ledger_index(Some(next_open_index));
        self.status_rpc_state
            .set_queue_report(Some(live_queue_report(&self.open_ledger, &self.tx_q)));

        Ok(next_open_index)
    }
}
