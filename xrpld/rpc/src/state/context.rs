//! JSON RPC context and runtime seams aligned with `xrpld/rpc/Context.h`.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use app::{ApplicationRoot, JobType, NetworkOpsOperatingMode, ServiceRegistry};
use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use overlay::Overlay;
use protocol::JsonValue;
use xrpl_core::PeerReservation;

use crate::state::role::Role;
use crate::state::tuning::Tuning;
use crate::status::{RpcErrorCode, Status};
use crate::{InfoSub, WsInfoSub};

#[derive(Debug, Default)]
struct RpcLedgerWalkJournal;

impl ledger::LedgerJournal for RpcLedgerWalkJournal {
    fn info(&self, message: &str) {
        tracing::info!(target: "rpc", "[ledger_request][walk] {message}");
    }

    fn warn(&self, message: &str) {
        tracing::warn!(target: "rpc", "[ledger_request][walk] {message}");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JsonContextHeaders<'a> {
    pub user: &'a str,
    pub forwarded_for: &'a str,
}

pub struct JsonContext<'a, Env> {
    pub params: &'a JsonValue,
    pub env: &'a Env,
    pub role: Role,
    pub api_version: u32,
    pub headers: JsonContextHeaders<'a>,
    pub unlimited: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RpcLoadType {
    #[default]
    Reference,
    MediumBurden,
    HeavyBurden,
    Exception,
}

/// Port of the tail of rippled `getOrAcquireLedger`: ask the shared
/// `InboundLedgers` registry (the same coordinator used by consensus and
/// ledger history) for the ledger. Like `InboundLedgers::acquire`, this never
/// blocks: an incomplete ledger starts or touches a background acquisition and
/// the RPC reports that it is still being acquired.
///
/// The previous implementation ran a private, synchronous acquisition with its
/// own 2048-entry TreeNodeCache and FullBelow cache. Because those caches did
/// not share nodes with resident ledgers, requesting a ledger that differs
/// from the tip materialized an entire second copy of the state tree in
/// memory from the NodeStore; on testnet that exhausted RAM and froze the
/// host. Sharing the node family bounds the work to the nodes that differ.
fn ledger_request_acquire(app: &ApplicationRoot, hash: Uint256, seq: u32) -> Status {
    if app
        .resolve_ledger_by_hash(SHAMapHash::new(hash))
        .is_some()
    {
        return Status::OK;
    }
    match acquire_generic(app, hash, seq) {
        Some(_) => Status::OK,
        None => Status::with_message(RpcErrorCode::NotReady, "acquiring ledger"),
    }
}

fn acquire_generic(app: &ApplicationRoot, hash: Uint256, seq: u32) -> Option<Arc<ledger::Ledger>> {
    let runtime = app.ledger_master_runtime()?;
    let registry = runtime
        .inbound_ledgers
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned())?;
    let ledger = registry.acquire(hash, seq, app::AcquireReason::Generic)?;
    // Match the previous cache-by-hash behaviour: an RPC acquisition must not
    // change the closed, validated or published ledger.
    runtime
        .ledger_master()
        .ledger_history()
        .insert(Arc::clone(&ledger), false);
    Some(ledger)
}

pub trait RpcRuntime {
    fn app(&self) -> Option<&ApplicationRoot> {
        None
    }

    fn network_ops_runtime(&self) -> Option<std::sync::Arc<app::AppNetworkOpsRuntime>> {
        self.app().and_then(|app| app.network_ops_runtime())
    }

    fn job_queue(&self) -> Option<app::JobQueue> {
        self.app().map(|app| app.job_queue().clone())
    }

    fn beta_rpc_api(&self) -> bool {
        false
    }

    fn client_job_count(&self) -> u32 {
        0
    }

    fn max_job_queue_clients(&self) -> u32 {
        Tuning::MAX_JOB_QUEUE_CLIENTS
    }

    fn has_current_ledger(&self) -> bool {
        true
    }

    fn has_closed_ledger(&self) -> bool {
        true
    }

    fn path_search_max(&self) -> u32 {
        0
    }

    fn path_search_old(&self) -> u32 {
        2
    }

    fn path_search(&self) -> u32 {
        2
    }

    fn path_search_fast(&self) -> u32 {
        2
    }

    fn network_synced(&self) -> bool {
        true
    }

    fn validated_ledger_age(&self) -> StdDuration {
        StdDuration::ZERO
    }

    fn current_ledger_index(&self) -> Option<u32> {
        None
    }

    /// Get the current open ledger for transaction simulation.
    /// Returns None if no ledger is available.
    fn current_ledger_for_simulation(&self) -> Option<std::sync::Arc<ledger::Ledger>> {
        None
    }

    fn standalone(&self) -> bool {
        false
    }

    fn ledger_accept(&self) -> Status {
        Status::new(RpcErrorCode::NotStandalone)
    }

    fn stop(&self) -> Status {
        Status::OK
    }

    fn peer_connect(&self, _ip: String, _port: u16) -> Status {
        Status::OK
    }

    fn peers_get(&self) -> JsonValue {
        protocol::json!({ "peers": [] })
    }

    fn can_delete_get(&self) -> u32 {
        0
    }

    fn can_delete_enabled(&self) -> bool {
        false
    }

    fn can_delete_last_rotated(&self) -> u32 {
        0
    }

    fn can_delete_seq_by_hash(&self, _hash: Uint256) -> Option<u32> {
        None
    }

    fn can_delete_set(&self, _seq: u32) -> Status {
        Status::OK
    }

    fn ledger_cleaner_trigger(&self, _params: &JsonValue) -> Status {
        Status::OK
    }

    fn ledger_request(&self, _seq: u32) -> Status {
        Status::OK
    }

    fn ledger_request_by_hash(&self, _hash: Uint256) -> Status {
        Status::new(RpcErrorCode::NotImplemented)
    }

    fn log_level_set(&self, _partition: String, _level: String) -> Status {
        Status::OK
    }

    fn log_level_get(&self) -> JsonValue {
        protocol::json!({ "levels": {} })
    }

    fn log_rotate(&self) -> Status {
        Status::OK
    }

    fn peer_reservations_add(
        &self,
        _public_key: protocol::PublicKey,
        _description: String,
    ) -> Status {
        Status::OK
    }

    fn peer_reservations_del(&self, _public_key: protocol::PublicKey) -> Status {
        Status::OK
    }

    fn peer_reservations_list(&self) -> JsonValue {
        protocol::json!({ "reservations": [] })
    }

    fn export_snapshot(&self, _output_path: &str) -> Result<JsonValue, String> {
        Err("Not implemented".to_owned())
    }

    fn snapshot_status(&self) -> JsonValue {
        protocol::json!({ "status": "success", "state": "unavailable" })
    }
}

impl RpcRuntime for () {}

impl RpcRuntime for ApplicationRoot {
    fn app(&self) -> Option<&ApplicationRoot> {
        Some(self)
    }

    fn client_job_count(&self) -> u32 {
        u32::try_from(self.job_queue().job_count_ge(JobType::JtClient)).unwrap_or(u32::MAX)
    }

    fn has_current_ledger(&self) -> bool {
        self.status_rpc_current_ledger_index().is_some()
            || self.live_current_ledger_index().is_some()
            || self.validated_ledger_seq().is_some()
            || self.closed_ledger_seq().is_some()
    }

    fn has_closed_ledger(&self) -> bool {
        self.closed_ledger().is_some()
    }

    fn path_search_max(&self) -> u32 {
        self.path_search_max()
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

    fn network_synced(&self) -> bool {
        // Matches reference `conditionMet`'s gate: `getOperatingMode() <
        // OperatingMode::SYNCING` is rejected, so SYNCING, TRACKING, and
        // FULL (displayed as "proposing" once active-validating) all count
        // as synced. Only DISCONNECTED/CONNECTED are rejected.
        self.network_ops_operating_mode() >= NetworkOpsOperatingMode::Syncing
    }

    fn validated_ledger_age(&self) -> StdDuration {
        ApplicationRoot::validated_ledger_age(self)
    }

    fn current_ledger_index(&self) -> Option<u32> {
        self.status_rpc_current_ledger_index()
            .or_else(|| self.live_current_ledger_index())
            .or_else(|| self.validated_ledger_seq().map(|seq| seq.saturating_add(1)))
    }

    fn current_ledger_for_simulation(&self) -> Option<std::sync::Arc<ledger::Ledger>> {
        self.closed_ledger().or_else(|| self.validated_ledger())
    }

    fn standalone(&self) -> bool {
        self.standalone()
    }

    fn ledger_accept(&self) -> Status {
        if !self.standalone() {
            return Status::new(RpcErrorCode::NotStandalone);
        }
        self.accept_standalone_ledger()
            .map(|_| Status::OK)
            .unwrap_or_else(|_| Status::new(RpcErrorCode::Internal))
    }

    fn stop(&self) -> Status {
        self.signal_stop("RPC stop command");
        Status::OK
    }

    fn peer_connect(&self, ip: String, port: u16) -> Status {
        if let Some(runtime) = self.overlay_runtime() {
            let address = format!("{}:{}", ip, port)
                .parse()
                .map_err(|_| Status::new(RpcErrorCode::InvalidParams));
            let address = match address {
                Ok(a) => a,
                Err(s) => return s,
            };
            let future = runtime.overlay().connect(address);
            tokio::spawn(async move {
                let _ = future.await;
            });
        }
        Status::OK
    }

    fn peers_get(&self) -> JsonValue {
        self.overlay_runtime()
            .as_ref()
            .map(|o| {
                let overlay = o.overlay();
                let peers = overlay.peers_json();
                protocol::json!({ "peers": peers })
            })
            .unwrap_or_else(|| protocol::json!({ "peers": [] }))
    }

    fn can_delete_get(&self) -> u32 {
        self.shamap_store_service()
            .map(|service| service.component().get_can_delete())
            .unwrap_or(0)
    }

    fn can_delete_enabled(&self) -> bool {
        self.shamap_store_service()
            .map(|service| service.component().advisory_delete())
            .unwrap_or(false)
    }

    fn can_delete_last_rotated(&self) -> u32 {
        self.shamap_store_service()
            .map(|service| service.component().get_last_rotated())
            .unwrap_or(0)
    }

    fn can_delete_seq_by_hash(&self, hash: Uint256) -> Option<u32> {
        self.ledger_master_runtime()
            .and_then(|runtime| {
                runtime
                    .ledger_master()
                    .get_ledger_by_hash(SHAMapHash::new(hash))
            })
            .map(|ledger| ledger.header().seq)
    }

    fn can_delete_set(&self, seq: u32) -> Status {
        match self.shamap_store_service() {
            Some(service) => match service.component().set_can_delete(seq) {
                Ok(_) => Status::OK,
                Err(_) => Status::new(RpcErrorCode::NotEnabled),
            },
            None => Status::new(RpcErrorCode::NotEnabled),
        }
    }

    fn ledger_cleaner_trigger(&self, params: &JsonValue) -> Status {
        let min_ledger = 0;
        let max_ledger = self
            .app()
            .and_then(|a| a.validated_ledger_seq())
            .unwrap_or(0);

        let mut request = ledger::LedgerCleanerRequest {
            validated_min: min_ledger,
            validated_max: max_ledger,
            ledger: None,
            min_ledger: Some(min_ledger),
            max_ledger: Some(max_ledger),
            full: None,
            fix_txns: None,
            check_nodes: None,
            stop: false,
        };

        if let JsonValue::Object(map) = params {
            if let Some(JsonValue::Unsigned(l)) = map.get("ledger") {
                request.ledger = Some(*l as u32);
            }
            if let Some(JsonValue::Unsigned(m)) = map.get("min_ledger") {
                request.min_ledger = Some(*m as u32);
            }
            if let Some(JsonValue::Unsigned(m)) = map.get("max_ledger") {
                request.max_ledger = Some(*m as u32);
            }
            if let Some(JsonValue::Bool(f)) = map.get("full") {
                request.full = Some(*f);
            }
            if let Some(JsonValue::Bool(f)) = map.get("fix_txns") {
                request.fix_txns = Some(*f);
            }
            if let Some(JsonValue::Bool(c)) = map.get("check_nodes") {
                request.check_nodes = Some(*c);
            }
            if let Some(JsonValue::Bool(s)) = map.get("stop") {
                request.stop = *s;
            }
        }

        if let Some(app) = self.app() {
            app.get_ledger_cleaner().clean(request);
        }
        Status::OK
    }

    fn ledger_request(&self, seq: u32) -> Status {
        // rippled doLedgerRequest -> getOrAcquireLedger (RPCLedgerHelpers.cpp).
        if seq == 0 {
            return Status::make_param_error("Ledger index too small");
        }
        // rippled: a sequence can only be resolved through a fresh validated
        // ledger (`getValidatedLedgerAge() > kMaxValidatedLedgerAge`).
        if self.validated_ledger_age() > StdDuration::from_secs(120) {
            return Status::new(RpcErrorCode::NotSynced);
        }
        let Some(validated) = self.validated_ledger() else {
            return Status::new(RpcErrorCode::NotSynced);
        };
        if seq >= validated.header().seq {
            return Status::make_param_error("Ledger index too large");
        }
        let journal = RpcLedgerWalkJournal;
        let needed = match validated.hash_of_seq(seq, &journal) {
            Some(hash) => hash,
            None => {
                // Find a ledger more likely to have the hash of the desired
                // ledger (getCandidateLedger: next 256-ledger boundary).
                let ref_index = seq.saturating_add(255) & !255u32;
                let Some(ref_hash) = validated.hash_of_seq(ref_index, &journal) else {
                    return Status::new(RpcErrorCode::LedgerNotFound);
                };
                let Some(reference) = self.resolve_ledger_by_hash(ref_hash) else {
                    let _ = acquire_generic(self, *ref_hash.as_uint256(), ref_index);
                    return Status::with_message(
                        RpcErrorCode::LedgerNotFound,
                        "acquiring ledger containing requested index",
                    );
                };
                match reference.hash_of_seq(seq, &journal) {
                    Some(hash) => hash,
                    None => return Status::new(RpcErrorCode::LedgerNotFound),
                }
            }
        };
        ledger_request_acquire(self, *needed.as_uint256(), seq)
    }

    fn ledger_request_by_hash(&self, hash: Uint256) -> Status {
        ledger_request_acquire(self, hash, 0)
    }

    fn log_level_set(&self, partition: String, level: String) -> Status {
        // Validate level
        let valid_levels = ["trace", "debug", "info", "warn", "error", "off"];
        let level_lower = level.to_ascii_lowercase();
        if !valid_levels.contains(&level_lower.as_str()) {
            return Status::new(crate::status::RpcErrorCode::InvalidParams);
        }

        // Validate partition characters (prevent filter injection)
        if partition != "base"
            && !partition.is_empty()
            && !partition
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == ':')
        {
            return Status::new(crate::status::RpcErrorCode::InvalidParams);
        }

        // Build filter: for "base"/empty set global level, for partition set only that module
        let filter = if partition == "base" || partition.is_empty() {
            level_lower
        } else {
            format!("info,{}={}", partition, level_lower)
        };

        match app::reload_log_filter(&filter) {
            Ok(()) => Status::OK,
            Err(e) => {
                tracing::warn!(target: "rpc", error = %e, "log_level_set failed");
                Status::new(crate::status::RpcErrorCode::InvalidParams)
            }
        }
    }

    fn log_level_get(&self) -> JsonValue {
        protocol::json!({ "levels": {} })
    }

    fn log_rotate(&self) -> Status {
        // Log rotation not yet in AppLogs
        Status::OK
    }

    fn peer_reservations_add(
        &self,
        public_key: protocol::PublicKey,
        description: String,
    ) -> Status {
        self.get_peer_reservations()
            .insert_or_assign(PeerReservation::new(public_key, description));
        Status::OK
    }

    fn peer_reservations_del(&self, public_key: protocol::PublicKey) -> Status {
        self.get_peer_reservations().erase(&public_key);
        Status::OK
    }

    fn peer_reservations_list(&self) -> JsonValue {
        let list = self.get_peer_reservations().list();
        protocol::json!({
            "reservations": list.into_iter().map(|r| r.to_json()).collect::<Vec<_>>()
        })
    }

    fn export_snapshot(&self, output_path: &str) -> Result<JsonValue, String> {
        use nodestore::snapshot::{
            SnapshotManifest, export_snapshot_with_cancellation, manifest::SNAPSHOT_VERSION,
        };
        use std::path::Path;

        let validated = self
            .validated_ledger()
            .ok_or_else(|| "No validated ledger available".to_owned())?;
        let header = validated.header();

        let node_store = self
            .node_store()
            .as_ref()
            .ok_or_else(|| "NodeStore not configured".to_owned())?;
        let backend = node_store
            .export_backend()
            .ok_or_else(|| "Backend not available for export".to_owned())?;

        let manifest = SnapshotManifest {
            version: SNAPSHOT_VERSION,
            ledger_seq: header.seq,
            ledger_hash: *header.hash.as_uint256().data(),
            account_hash: *header.account_hash.as_uint256().data(),
            tx_hash: *header.tx_hash.as_uint256().data(),
            parent_hash: *header.parent_hash.as_uint256().data(),
            drops: header.drops,
            close_time: header.close_time,
            parent_close_time: header.parent_close_time,
            close_time_res: header.close_time_resolution,
            close_flags: header.close_flags,
            chunks: Vec::new(),
        };

        let ledger_seq = header.seq;
        let ledger_hash = header.hash.to_string();
        let account_hash = header.account_hash.to_string();
        let output_owned = output_path.to_owned();
        self.start_snapshot_export(
            output_owned.clone(),
            ledger_seq,
            move |export_state, cancellation| {
                std::thread::Builder::new()
                    .name(format!("snapshot-export-{ledger_seq}"))
                    .spawn(move || {
                        let path = Path::new(&output_owned);
                        tracing::info!(
                            target: "snapshot",
                            ledger_seq,
                            path = %path.display(),
                            "Background snapshot export started"
                        );
                        match export_snapshot_with_cancellation(
                            backend.as_ref(),
                            &manifest,
                            path,
                            &cancellation,
                        ) {
                            Ok(()) => {
                                let file_size = std::fs::metadata(path)
                                    .map(|metadata| metadata.len())
                                    .unwrap_or(0);
                                export_state.complete(file_size);
                                tracing::info!(
                                    target: "snapshot",
                                    ledger_seq,
                                    path = %path.display(),
                                    file_size,
                                    "Snapshot export completed successfully"
                                );
                            }
                            Err(e) => {
                                export_state.fail(e.to_string());
                                tracing::error!(
                                    target: "snapshot",
                                    error = %e,
                                    ledger_seq,
                                    "Snapshot export failed"
                                );
                            }
                        }
                    })
                    .map_err(|error| format!("Failed to spawn export thread: {error}"))
            },
        )?;

        Ok(protocol::json!({
            "status": "started",
            "message": "Snapshot export running in background. Monitor progress in logs.",
            "ledger_seq": ledger_seq,
            "ledger_hash": ledger_hash,
            "account_hash": account_hash,
            "output": output_path
        }))
    }

    fn snapshot_status(&self) -> JsonValue {
        let snapshot = self.snapshot_export_status();
        let mut result = std::collections::BTreeMap::new();
        result.insert("status".to_owned(), JsonValue::String("success".to_owned()));
        result.insert(
            "state".to_owned(),
            JsonValue::String(snapshot.phase.as_str().to_owned()),
        );
        if let Some(output) = snapshot.output {
            result.insert("output".to_owned(), JsonValue::String(output));
        }
        if let Some(ledger_seq) = snapshot.ledger_seq {
            result.insert(
                "ledger_seq".to_owned(),
                JsonValue::Unsigned(u64::from(ledger_seq)),
            );
        }
        if let Some(file_size) = snapshot.file_size {
            result.insert("file_size".to_owned(), JsonValue::Unsigned(file_size));
        }
        if let Some(error) = snapshot.error {
            result.insert("error".to_owned(), JsonValue::String(error));
        }
        JsonValue::Object(result)
    }
}

impl crate::commands::black_list::BlackListSource for ApplicationRoot {
    fn black_list_json(&self) -> JsonValue {
        JsonValue::from(self.get_resource_manager().get_json())
    }

    fn black_list_json_with_threshold(&self, threshold: i64) -> JsonValue {
        JsonValue::from(
            self.get_resource_manager()
                .get_json_with_threshold(threshold),
        )
    }
}

pub struct RpcRequestContext<'a, Env, Runtime = ()> {
    pub params: &'a JsonValue,
    pub env: &'a Env,
    pub runtime: &'a Runtime,
    pub role: Role,
    pub api_version: u32,
    pub headers: JsonContextHeaders<'a>,
    pub request_headers: BTreeMap<String, String>,
    pub unlimited: bool,
    pub remote_ip: Option<IpAddr>,
    pub load_type: RpcLoadType,
}

impl<'a, Env, Runtime> RpcRequestContext<'a, Env, Runtime> {
    pub fn json_context(&self) -> JsonContext<'a, Env> {
        if let Some(client_ip) = self.remote_ip {
            tracing::debug!(target: "rpc", role = ?self.role, ip = %client_ip, "RPC access check");
        }
        JsonContext {
            params: self.params,
            env: self.env,
            role: self.role,
            api_version: self.api_version,
            headers: self.headers,
            unlimited: self.unlimited,
        }
    }

    pub fn websocket_session(&self, remote_endpoint: SocketAddr) -> WsInfoSub {
        WsInfoSub::from_request(
            InfoSub::new(self.role),
            remote_endpoint,
            self.request_headers.clone(),
            self.api_version,
            (!self.headers.user.is_empty()).then_some(self.headers.user),
            (!self.headers.forwarded_for.is_empty()).then_some(self.headers.forwarded_for),
        )
    }

    pub fn remote_ip_or_internal(&self) -> Result<IpAddr, Status> {
        self.remote_ip
            .ok_or_else(|| Status::new(RpcErrorCode::Internal))
    }
}
