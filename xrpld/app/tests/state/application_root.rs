use app::{
    ApplicationRoot, ApplicationRootOptions, GrpcRuntime, LoadFeeControl, LoadManagerJournal,
    MainRuntime, ManagedComponent, NetworkOpsOperatingMode, OverlayStatusSnapshot,
    OverlayStatusSource, PublishedGrpcPort, PublishedServerPort, RuntimeBindings,
    SERVER_OKAY_NEED_NETWORK_LEDGER_REASON, SERVER_OKAY_NOT_SYNCED_REASON,
    SERVER_OKAY_TOO_MUCH_LOAD_REASON, SERVER_OKAY_UNL_BLOCKED_REASON, SHAMapStore,
    SHAMapStoreCloseTimeProvider, SHAMapStoreComponent, SHAMapStoreComponentRuntime,
    SHAMapStoreHealthRuntime, SHAMapStoreOperatingMode, SHAMapStoreRuntime, SHAMapStoreService,
    ServerPortClientSetup, ServerPortOverlaySetup, ServerPortSetup, ServerPortsSetup,
    ServiceRegistry, SharedSHAMapStoreHealthState, StatusMetricsSource, StatusRpcGitInfo,
    StatusRpcLastClose, TransStatus, Transaction,
};
use basics::base_uint::Uint256;
use ledger::{AcceptedLedger, LEDGER_DEFAULT_TIME_RESOLUTION, Ledger, LedgerHeader};
use overlay::{Overlay, OverlayHandoff, OverlayImpl, Peer, PeerImp, Setup};
use protocol::{
    JsonOptions, JsonValue, LedgerEntryType, STAmount, STArray, STLedgerEntry, STObject, STTx,
    STVector256, TxType, amendments_key, calc_account_id, get_field_by_symbol,
};
use protocol::{KeyType, PublicKey, SecretKey, derive_public_key};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use serde_json::json;
use shamap::{
    item::SHAMapItem, mutation::MutableTree, sync::SHAMapType, sync::SyncState, sync::SyncTree,
    tree_node::SHAMapNodeType,
};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tx::{QueueTxQRpcDrops, QueueTxQRpcLevels, QueueTxQRpcReport};
use xrpl_core::PeerReservation;

#[derive(Default)]
struct RecordingComponent {
    name: &'static str,
    fail_on_start: bool,
    events: Arc<Mutex<Vec<String>>>,
    fd_required: usize,
}

impl RecordingComponent {
    fn new(name: &'static str, events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            name,
            fail_on_start: false,
            events,
            fd_required: 8,
        }
    }

    fn with_fd_required(
        name: &'static str,
        events: Arc<Mutex<Vec<String>>>,
        fd_required: usize,
    ) -> Self {
        Self {
            name,
            fail_on_start: false,
            events,
            fd_required,
        }
    }

    fn failing(name: &'static str, events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            name,
            fail_on_start: true,
            events,
            fd_required: 8,
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
        self.fd_required
    }
}

fn ledger_with_amendments(
    ledger_seq: u32,
    close_time: u32,
    enabled: &[Uint256],
    majorities: &[(Uint256, u32)],
) -> Ledger {
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::Amendments, amendments_key());
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), Uint256::from_u64(1));
    entry.set_field_u32(
        get_field_by_symbol("sfPreviousTxnLgrSeq"),
        ledger_seq.saturating_sub(1),
    );
    if !enabled.is_empty() {
        entry.set_field_v256(
            get_field_by_symbol("sfAmendments"),
            STVector256::from_values(get_field_by_symbol("sfAmendments"), enabled.to_vec()),
        );
    }
    if !majorities.is_empty() {
        let mut array = STArray::new(get_field_by_symbol("sfMajorities"));
        for &(amendment, majority_close_time) in majorities {
            let mut majority = STObject::new(get_field_by_symbol("sfMajority"));
            majority.set_field_h256(get_field_by_symbol("sfAmendment"), amendment);
            majority.set_field_u32(get_field_by_symbol("sfCloseTime"), majority_close_time);
            array.push_back(majority);
        }
        entry.set_field_array(get_field_by_symbol("sfMajorities"), array);
    }

    let mut tree = MutableTree::new(1);
    tree.add_item(
        SHAMapNodeType::AccountState,
        SHAMapItem::new(amendments_key(), entry.get_serializer().data().to_vec()),
    )
    .expect("amendments entry should insert");

    let state_map = SyncTree::from_root_with_type(
        tree.root(),
        SHAMapType::State,
        false,
        ledger_seq,
        SyncState::Modifying,
    );

    let mut ledger = Ledger::from_maps(
        LedgerHeader {
            seq: ledger_seq,
            close_time,
            parent_close_time: close_time.saturating_sub(5),
            close_time_resolution: LEDGER_DEFAULT_TIME_RESOLUTION,
            ..LedgerHeader::default()
        },
        state_map,
        SyncTree::new_with_type(SHAMapType::Transaction, false, ledger_seq),
    );
    ledger.set_immutable(false);
    ledger
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

#[derive(Default)]
struct ServiceRuntime;

impl SHAMapStoreRuntime for ServiceRuntime {
    fn start_background_work(&mut self) {}

    fn stop_background_work(&mut self) {}

    fn minimum_sql_seq(&self) -> Option<u32> {
        None
    }
}

impl SHAMapStoreHealthRuntime for ServiceRuntime {
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

impl SHAMapStoreComponentRuntime for ServiceRuntime {}

struct TestHandoff;

impl OverlayHandoff for TestHandoff {
    fn on_handoff(
        &self,
        _request: &http::Request<()>,
        _remote_address: SocketAddr,
    ) -> overlay::Handoff {
        overlay::Handoff::Accepted
    }
}

#[derive(Debug)]
struct NoVerify;

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

fn overlay_setup(network_id: Option<u32>) -> Setup {
    static TLS_PROVIDER: OnceLock<()> = OnceLock::new();
    TLS_PROVIDER.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    Setup {
        client_config: Some(Arc::new(
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoVerify))
                .with_no_client_auth(),
        )),
        server_config: None,
        public_ip: None,
        ip_limit: 0,
        peer_limit: 0,
        verify_endpoints: true,
        crawl_options: 0,
        network_id,
        fixed_peer_ips: std::collections::HashSet::new(),
        vl_enabled: true,
        tx_reduce_relay_enabled: true,
        tx_reduce_relay_min_peers: 1,
        tx_relay_percentage: 0,
        vp_reduce_relay_base_squelch_enabled: true,
        vp_reduce_relay_max_selected_peers: 3,
        reduce_relay_wait: Duration::from_secs(0),
    }
}

fn overlay_public_key(seed: u8) -> PublicKey {
    let secret = SecretKey::from_bytes([seed; 32]);
    derive_public_key(KeyType::Secp256k1, &secret).expect("public key")
}

fn overlay_peer(id: u32, seed: u8) -> Arc<PeerImp> {
    PeerImp::new(
        id,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5000 + id as u16),
        overlay_public_key(seed),
        format!("peer-{id}"),
    )
}

#[derive(Debug, Clone)]
struct FixedStatusMetricsSource {
    counters: serde_json::Value,
    current_activities: serde_json::Value,
    nodestore: serde_json::Value,
    state_accounting: serde_json::Value,
    server_state_duration_us: Option<String>,
    initial_sync_duration_us: Option<String>,
}

impl StatusMetricsSource for FixedStatusMetricsSource {
    fn counters_json(&self) -> serde_json::Value {
        self.counters.clone()
    }

    fn current_activities_json(&self) -> serde_json::Value {
        self.current_activities.clone()
    }

    fn nodestore_counts_json(&self) -> serde_json::Value {
        self.nodestore.clone()
    }

    fn state_accounting_json(&self) -> serde_json::Value {
        self.state_accounting.clone()
    }

    fn server_state_duration_us(&self) -> Option<String> {
        self.server_state_duration_us.clone()
    }

    fn initial_sync_duration_us(&self) -> Option<String> {
        self.initial_sync_duration_us.clone()
    }
}

fn account(fill: u8) -> protocol::AccountID {
    protocol::AccountID::from_array([fill; 20])
}

fn payment_tx(sequence: u32, fill: u8) -> STTx {
    STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(fill));
        tx.set_account_id(
            get_field_by_symbol("sfDestination"),
            account(fill.wrapping_add(1)),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    })
}

fn signed_invalid_payment_tx(seed: u8) -> Arc<STTx> {
    let secret = SecretKey::from_bytes([seed; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    let source = calc_account_id(public.as_bytes());
    let mut tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), source);
        tx.set_account_id(
            get_field_by_symbol("sfDestination"),
            account(seed.wrapping_add(1)),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 0);
        tx.set_field_vl(get_field_by_symbol("sfSigningPubKey"), public.as_bytes());
    });
    tx.sign(&public, &secret, None)
        .expect("signature should succeed");
    Arc::new(tx)
}

#[test]
fn application_root_binds_runtime_components_under_one_shell() {
    let mut root = ApplicationRoot::new(0).expect("root shell should build");
    let events = Arc::new(Mutex::new(Vec::new()));
    let ledger = Arc::new(RecordingComponent::new("ledger", Arc::clone(&events)));
    let nodestore = Arc::new(RecordingComponent::new("nodestore", Arc::clone(&events)));
    let consensus = Arc::new(RecordingComponent::new("consensus", Arc::clone(&events)));
    let server = Arc::new(RecordingComponent::new("server", Arc::clone(&events)));
    let overlay = Arc::new(RecordingComponent::new("overlay", Arc::clone(&events)));
    let shamap_store = Arc::new(RecordingComponent::new("shamap_store", Arc::clone(&events)));

    root.set_runtime_bindings(RuntimeBindings {
        ledger: Some(ledger.clone()),
        nodestore: Some(nodestore.clone()),
        shamap_store: Some(shamap_store.clone()),
        consensus: Some(consensus.clone()),
        server: Some(server.clone()),
        resolver: None,
        overlay: Some(overlay.clone()),
        validator_site: None,
        perf_log: None,
        grpc: GrpcRuntime::DisabledExplicit {
            reason: "disabled until async gRPC parity is landed".to_owned(),
        },
    });

    let runtime = MainRuntime::new(root);
    runtime.start().expect("runtime should start");
    runtime.signal_stop("test");
    runtime.shutdown();

    assert_eq!(
        events.lock().expect("events mutex").as_slice(),
        &[
            "nodestore:start".to_owned(),
            "shamap_store:start".to_owned(),
            "overlay:start".to_owned(),
            "ledger:start".to_owned(),
            "consensus:start".to_owned(),
            "server:start".to_owned(),
            "shamap_store:stop".to_owned(),
            "overlay:stop".to_owned(),
            "consensus:stop".to_owned(),
            "server:stop".to_owned(),
            "ledger:stop".to_owned(),
            "nodestore:stop".to_owned(),
        ]
    );
    assert!(matches!(
        &runtime.root().runtime_bindings().grpc,
        GrpcRuntime::DisabledExplicit { .. }
    ));
    assert_eq!(runtime.root().runtime_bindings().fd_required(), 48);
    assert_eq!(runtime.root().fd_required(), 1024);

    assert_eq!(
        runtime
            .start()
            .expect_err("shutdown runtime should not restart"),
        "runtime has already been shut down"
    );
}

#[test]
fn application_root_can_be_built_with_runtime_bindings_and_report_fd_budget() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let root = ApplicationRoot::with_runtime_bindings(
        ApplicationRootOptions {
            io_threads: 2,
            job_queue_threads: 2,
            ..ApplicationRootOptions::default()
        },
        RuntimeBindings {
            ledger: Some(Arc::new(RecordingComponent::new(
                "ledger",
                Arc::clone(&events),
            ))),
            server: Some(Arc::new(RecordingComponent::new(
                "server",
                Arc::clone(&events),
            ))),
            grpc: GrpcRuntime::DisabledExplicit {
                reason: "disabled for bootstrap parity test".to_owned(),
            },
            ..RuntimeBindings::default()
        },
    )
    .expect("root shell should build");

    assert_eq!(root.fd_required(), 1024);
    assert!(root.runtime_bindings().ledger.is_some());
    assert!(root.runtime_bindings().server.is_some());
    assert!(matches!(
        &root.runtime_bindings().grpc,
        GrpcRuntime::DisabledExplicit { .. }
    ));
}

#[test]
fn application_root_owns_transaction_master_cache_like_application_cpp() {
    let root = ApplicationRoot::new(0).expect("root shell should build");
    let tx = payment_tx(5, 0x41);
    let mut shared = Arc::new(Mutex::new(Transaction::new(Arc::new(tx.clone()))));

    root.canonicalize_transaction(&mut shared);

    let cached = root
        .fetch_cached_transaction(&tx.get_transaction_id())
        .expect("root-owned transaction master should cache canonicalized transactions");

    assert!(Arc::ptr_eq(&shared, &cached));
    assert_eq!(root.transaction_master().get_cache().size(), 1);
}

#[test]
fn application_root_owns_real_inbound_transaction_sets_like_application_cpp() {
    let root = ApplicationRoot::new(0).expect("root shell should build");
    {
        let mut inbound = root
            .inbound_transactions()
            .lock()
            .expect("inbound transactions mutex");

        assert!(inbound.get_set(Uint256::zero(), false).is_some());
        assert_eq!(inbound.len(), 1);

        inbound.new_round(25);
        assert!(inbound.get_set(Uint256::from_u64(7001), false).is_none());
    }

    assert_eq!(
        root.get_inbound_transactions()
            .lock()
            .expect("inbound transactions mutex")
            .len(),
        1
    );
}

#[test]
fn application_root_owns_real_accepted_ledger_cache_like_application_cpp() {
    let root = ApplicationRoot::new(0).expect("root shell should build");
    let ledger = Arc::new(Ledger::from_ledger_seq_and_close_time(88, 777, false));
    let accepted =
        Arc::new(AcceptedLedger::new(Arc::clone(&ledger)).expect("accepted ledger should build"));
    let hash = *ledger.header().hash.as_uint256();

    assert_eq!(root.accepted_ledger_cache().name(), "AcceptedLedger");
    assert_eq!(root.accepted_ledger_cache().size(), 0);
    assert!(
        !root
            .accepted_ledger_cache()
            .insert(hash, Arc::clone(&accepted))
    );

    let cached = root
        .get_accepted_ledger_cache()
        .retrieve(&hash)
        .expect("accepted ledger should be cached");
    assert_eq!(cached.get_ledger().header().seq, 88);
}

#[test]
fn application_root_shapes_transaction_json_from_owned_close_time_state() {
    let root = ApplicationRoot::new(0).expect("root shell should build");
    root.on_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
        91, 777, false,
    )));

    let mut transaction = Transaction::new(Arc::new(payment_tx(6, 0x51)));
    transaction.set_status_with_ledger(TransStatus::COMMITTED, 91, Some(2), Some(3));

    let JsonValue::Object(json) =
        root.transaction_json(&transaction, JsonOptions::INCLUDE_DATE, false)
    else {
        panic!("transaction JSON should remain an object");
    };

    assert_eq!(json.get("ledger_index"), Some(&JsonValue::Unsigned(91)));
    assert_eq!(json.get("date"), Some(&JsonValue::Signed(777)));
    assert_eq!(
        json.get("ctid"),
        Some(&JsonValue::String("C000005B00020003".to_owned()))
    );
}

#[test]
fn application_root_fd_budget_honors_larger_runtime_binding_requirements() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let root = ApplicationRoot::with_runtime_bindings(
        ApplicationRootOptions::default(),
        RuntimeBindings {
            ledger: Some(Arc::new(RecordingComponent::with_fd_required(
                "ledger",
                Arc::clone(&events),
                1400,
            ))),
            server: Some(Arc::new(RecordingComponent::with_fd_required(
                "server",
                Arc::clone(&events),
                2200,
            ))),
            grpc: GrpcRuntime::DisabledExplicit {
                reason: "disabled for bootstrap parity test".to_owned(),
            },
            ..RuntimeBindings::default()
        },
    )
    .expect("root shell should build");

    assert_eq!(root.runtime_bindings().fd_required(), 3600);
    assert_eq!(root.fd_required(), 3600);
}

#[test]
fn application_root_rolls_back_started_components_if_startup_fails() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut root = ApplicationRoot::new(0).expect("root shell should build");
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
fn application_root_routes_live_shamap_store_health_and_validated_ledger_notifications() {
    let mut root = ApplicationRoot::new(0).expect("root shell should build");
    let close_time = Arc::new(FixedCloseTimeProvider::new(120));
    let health = Arc::new(SharedSHAMapStoreHealthState::new(close_time.clone()));
    let component = Arc::new(SHAMapStoreComponent::new(
        SHAMapStore::new(256, false, 9),
        Box::new(ServiceRuntime),
        None,
    ));
    let service = Arc::new(SHAMapStoreService::new(component.clone(), health.clone()));

    assert!(root.attach_shamap_store_service(service).is_none());
    assert!(root.set_shamap_store_operating_mode(SHAMapStoreOperatingMode::Full));
    assert!(
        root.on_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
            1_156, 100, false,
        )))
    );

    assert_eq!(health.operating_mode(), SHAMapStoreOperatingMode::Full);
    assert_eq!(
        root.shamap_store_operating_mode(),
        Some(SHAMapStoreOperatingMode::Full)
    );
    assert_eq!(component.snapshot().queued_ledger_seq(), Some(1_156));
    assert_eq!(root.validated_ledger_seq(), Some(1_156));
    assert_eq!(health.validated_ledger_age(), Duration::from_secs(20));

    close_time.now_close_time.store(127, Ordering::Release);
    assert_eq!(health.validated_ledger_age(), Duration::from_secs(27));
}

#[test]
fn application_root_service_wiring_refreshes_live_state_and_marks_shutdown_in_health() {
    let mut root = ApplicationRoot::new(0).expect("root shell should build");
    let close_time = Arc::new(FixedCloseTimeProvider::new(120));
    let health = Arc::new(SharedSHAMapStoreHealthState::new(close_time.clone()));
    let component = Arc::new(SHAMapStoreComponent::new(
        SHAMapStore::new(256, false, 9),
        Box::new(ServiceRuntime),
        None,
    ));
    let service = Arc::new(SHAMapStoreService::new(component.clone(), health.clone()));

    assert!(root.attach_shamap_store_service(service).is_none());

    let runtime = MainRuntime::new(root);
    runtime.start().expect("runtime should start");

    assert!(!health.is_stopping());
    assert!(
        runtime
            .root()
            .set_shamap_store_operating_mode(SHAMapStoreOperatingMode::Other)
    );
    assert_eq!(health.operating_mode(), SHAMapStoreOperatingMode::Other);
    assert!(
        runtime
            .root()
            .set_shamap_store_operating_mode(SHAMapStoreOperatingMode::Full)
    );
    assert_eq!(health.operating_mode(), SHAMapStoreOperatingMode::Full);

    assert!(
        runtime
            .root()
            .on_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
                1_156, 100, false,
            )))
    );
    assert_eq!(runtime.root().validated_ledger_seq(), Some(1_156));
    assert_eq!(health.validated_ledger_age(), Duration::from_secs(20));

    assert!(
        runtime
            .root()
            .on_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
                1_157, 118, false,
            )))
    );
    assert_eq!(runtime.root().validated_ledger_seq(), Some(1_157));
    assert_eq!(health.validated_ledger_age(), Duration::from_secs(2));

    assert!(runtime.signal_stop("test"));
    assert!(
        !health.is_stopping(),
        "signal_stop should not mark the managed service stopped before shutdown"
    );

    close_time.now_close_time.store(129, Ordering::Release);
    assert_eq!(health.validated_ledger_age(), Duration::from_secs(11));

    runtime.shutdown();
    assert!(health.is_stopping());
}

#[test]
fn application_root_keeps_status_rpc_snapshot_under_app_ownership() {
    let root = ApplicationRoot::new(0).expect("root shell should build");

    assert_eq!(root.status_rpc_current_ledger_index(), None);
    assert_eq!(root.status_rpc_queue_report(), None);

    assert_eq!(root.set_status_rpc_current_ledger_index(Some(91_234)), None);
    assert_eq!(
        root.set_status_rpc_queue_report(Some(QueueTxQRpcReport {
            ledger_current_index: 91_234,
            expected_ledger_size: "32".to_owned(),
            current_ledger_size: "31".to_owned(),
            current_queue_size: "4".to_owned(),
            max_queue_size: Some("200".to_owned()),
            levels: QueueTxQRpcLevels {
                reference_level: "256".to_owned(),
                minimum_level: "300".to_owned(),
                median_level: "400".to_owned(),
                open_ledger_level: "500".to_owned(),
            },
            drops: QueueTxQRpcDrops {
                base_fee: "10".to_owned(),
                median_fee: "16".to_owned(),
                minimum_fee: "12".to_owned(),
                open_ledger_fee: "20".to_owned(),
            },
        })),
        None
    );

    assert_eq!(root.status_rpc_current_ledger_index(), Some(91_234));
    assert_eq!(
        root.status_rpc_queue_report()
            .expect("queue report should exist"),
        QueueTxQRpcReport {
            ledger_current_index: 91_234,
            expected_ledger_size: "32".to_owned(),
            current_ledger_size: "31".to_owned(),
            current_queue_size: "4".to_owned(),
            max_queue_size: Some("200".to_owned()),
            levels: QueueTxQRpcLevels {
                reference_level: "256".to_owned(),
                minimum_level: "300".to_owned(),
                median_level: "400".to_owned(),
                open_ledger_level: "500".to_owned(),
            },
            drops: QueueTxQRpcDrops {
                base_fee: "10".to_owned(),
                median_fee: "16".to_owned(),
                minimum_fee: "12".to_owned(),
                open_ledger_fee: "20".to_owned(),
            },
        }
    );
}

#[test]
fn application_root_tracks_network_ops_mode_strings_state_names() {
    let root = ApplicationRoot::new(0).expect("root shell should build");

    assert_eq!(
        root.network_ops_operating_mode(),
        NetworkOpsOperatingMode::Disconnected
    );
    assert_eq!(root.network_ops_operating_mode_string(), "disconnected");

    // Use direct state set to test mode strings without normalization interference
    root.network_ops_state()
        .set_operating_mode(NetworkOpsOperatingMode::Syncing);
    assert_eq!(
        root.network_ops_operating_mode(),
        NetworkOpsOperatingMode::Syncing
    );
    assert_eq!(root.network_ops_operating_mode_string(), "syncing");
}

#[test]
fn application_root_tracks_path_search_tuning_config_values() {
    let mut root = ApplicationRoot::new(0).expect("root shell should build");

    assert_eq!(root.path_search_old(), 2);
    assert_eq!(root.path_search(), 2);
    assert_eq!(root.path_search_fast(), 2);
    assert_eq!(root.path_search_max(), 3);

    root.set_path_search_levels(5, 6, 4);
    let previous_max = root.set_path_search_max(9);

    assert_eq!(previous_max, 3);
    assert_eq!(root.path_search_old(), 5);
    assert_eq!(root.path_search(), 6);
    assert_eq!(root.path_search_fast(), 4);
    assert_eq!(root.path_search_max(), 9);
}

#[test]
fn application_root_tracks_ledger_master_owner_state_without_service() {
    let root = ApplicationRoot::new(0).expect("root shell should build");
    root.on_closed_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_154, 95, false,
    )));
    assert!(
        root.on_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
            1_156, 100, false,
        )))
    );
    root.on_published_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_155, 99, false,
    )));
    let now_close_time = root.time_keeper().close_time().as_seconds();
    root.ledger_master_state()
        .set_validated_close_time(now_close_time.saturating_sub(20));
    root.ledger_master_state()
        .set_published_close_time(now_close_time.saturating_sub(21));

    assert_eq!(root.closed_ledger_seq(), Some(1_154));
    assert_eq!(root.validated_ledger_seq(), Some(1_156));
    assert_eq!(root.published_ledger_seq(), Some(1_155));
    assert_eq!(root.validated_ledger_age(), Duration::from_secs(20));
}

#[test]
fn application_root_server_okay_matches_current_elb_gate_order() {
    let root = ApplicationRoot::with_options(app::ApplicationRootOptions {
        elb_support: true,
        ..app::ApplicationRootOptions::default()
    })
    .expect("root shell should build");

    assert_eq!(root.server_okay(), Err(SERVER_OKAY_NOT_SYNCED_REASON));

    root.set_need_network_ledger(true);
    assert_eq!(
        root.server_okay(),
        Err(SERVER_OKAY_NEED_NETWORK_LEDGER_REASON)
    );

    root.set_need_network_ledger(false);
    root.set_unl_blocked(true);
    assert_eq!(root.server_okay(), Err(SERVER_OKAY_UNL_BLOCKED_REASON));

    root.set_unl_blocked(false);
    root.set_network_ops_operating_mode(NetworkOpsOperatingMode::Full);
    root.on_published_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_155, 99, false,
    )));
    let now_close_time = root.time_keeper().close_time().as_seconds();
    root.ledger_master_state()
        .set_published_close_time(now_close_time.saturating_sub(21));
    assert_eq!(root.server_okay(), Err("No published ledger"));

    root.on_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_156, 100, false,
    )));
    root.ledger_master_state()
        .set_published_close_time(now_close_time.saturating_sub(21));
    root.ledger_master_state()
        .set_validated_close_time(now_close_time.saturating_sub(20));

    assert_eq!(root.server_okay(), Ok(()));

    assert!(!root.load_fee_track().raise_local_fee());
    assert!(root.load_fee_track().raise_local_fee());
    assert_eq!(root.server_okay(), Err(SERVER_OKAY_TOO_MUCH_LOAD_REASON));
}

#[test]
fn application_root_can_build_shamap_store_service_from_its_own_time_keeper() {
    let mut root = ApplicationRoot::new(0).expect("root shell should build");
    let component = Arc::new(SHAMapStoreComponent::new(
        SHAMapStore::new(256, false, 9),
        Box::new(ServiceRuntime),
        None,
    ));

    let service = root.attach_shamap_store_component(component.clone());

    assert!(root.shamap_store_service().is_some());
    assert!(root.runtime_bindings().shamap_store.is_some());
    assert_eq!(service.validated_ledger_seq(), None);
    assert_eq!(service.component().fd_required(), component.fd_required());
}

#[test]
fn application_root_owned_service_reads_live_network_ops_mode() {
    let mut root = ApplicationRoot::new(0).expect("root shell should build");
    let component = Arc::new(SHAMapStoreComponent::new(
        SHAMapStore::new(256, false, 9),
        Box::new(ServiceRuntime),
        None,
    ));

    let service = root.attach_shamap_store_component(component);

    assert_eq!(service.operating_mode(), SHAMapStoreOperatingMode::Other);

    root.set_network_ops_operating_mode(NetworkOpsOperatingMode::Tracking);
    assert_eq!(service.operating_mode(), SHAMapStoreOperatingMode::Other);

    root.set_network_ops_operating_mode(NetworkOpsOperatingMode::Full);
    assert_eq!(service.operating_mode(), SHAMapStoreOperatingMode::Full);
}

#[test]
fn application_root_owned_service_reads_validated_age_from_root_ledger_master_state() {
    let mut root = ApplicationRoot::new(0).expect("root shell should build");
    let component = Arc::new(SHAMapStoreComponent::new(
        SHAMapStore::new(256, false, 9),
        Box::new(ServiceRuntime),
        None,
    ));

    let service = root.attach_shamap_store_component(component);
    root.on_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_156, 100, false,
    )));
    let now_close_time = root.time_keeper().close_time().as_seconds();
    root.ledger_master_state()
        .set_validated_close_time(now_close_time.saturating_sub(20));

    assert_eq!(service.validated_ledger_seq(), Some(1_156));
    assert_eq!(
        service.health().validated_ledger_age(),
        root.validated_ledger_age()
    );
}

fn sample_status_queue_report(ledger_current_index: u32) -> QueueTxQRpcReport {
    QueueTxQRpcReport {
        ledger_current_index,
        expected_ledger_size: "14".to_owned(),
        current_ledger_size: "11".to_owned(),
        current_queue_size: "3".to_owned(),
        max_queue_size: Some("20".to_owned()),
        levels: QueueTxQRpcLevels {
            reference_level: "256".to_owned(),
            minimum_level: "128".to_owned(),
            median_level: "192".to_owned(),
            open_ledger_level: "384".to_owned(),
        },
        drops: QueueTxQRpcDrops {
            base_fee: "10".to_owned(),
            median_fee: "20".to_owned(),
            minimum_fee: "15".to_owned(),
            open_ledger_fee: "30".to_owned(),
        },
    }
}

#[test]
fn application_root_owns_narrow_status_rpc_state_for_later_rpc_reads() {
    let root = ApplicationRoot::new(0).expect("root shell should build");

    assert_eq!(root.status_rpc_current_ledger_index(), None);
    assert_eq!(root.status_rpc_queue_report(), None);
    assert_eq!(root.status_rpc_peer_count(), None);
    assert_eq!(root.status_rpc_network_id(), None);
    assert_eq!(root.status_rpc_last_close(), None);
    assert_eq!(root.status_rpc_hostid(), None);
    assert_eq!(root.status_rpc_server_domain(), None);
    assert_eq!(root.status_rpc_node_size(), None);
    assert_eq!(root.status_rpc_io_latency_ms(), None);
    assert_eq!(root.status_rpc_complete_ledgers(), None);
    assert_eq!(root.status_rpc_fetch_pack(), None);
    assert_eq!(root.status_rpc_git_info(), None);

    let first = sample_status_queue_report(777);
    let last_close = StatusRpcLastClose {
        proposers: 5,
        converge_time: Duration::from_millis(850),
    };
    assert_eq!(root.set_status_rpc_current_ledger_index(Some(778)), None);
    assert_eq!(root.set_status_rpc_queue_report(Some(first.clone())), None);
    assert_eq!(root.set_status_rpc_peer_count(Some(12)), None);
    assert_eq!(root.set_status_rpc_network_id(Some(21337)), None);
    assert_eq!(
        root.set_status_rpc_last_close(Some(last_close.clone())),
        None
    );
    assert_eq!(
        root.set_status_rpc_hostid(Some("host-one".to_owned())),
        None
    );
    assert_eq!(
        root.set_status_rpc_server_domain(Some("example.com".to_owned())),
        None
    );
    assert_eq!(root.set_status_rpc_node_size(Some("huge".to_owned())), None);
    assert_eq!(root.set_status_rpc_io_latency_ms(Some(17)), None);
    assert_eq!(
        root.set_status_rpc_complete_ledgers(Some("32570-918244".to_owned())),
        None
    );
    assert_eq!(root.set_status_rpc_fetch_pack(Some(9)), None);
    assert_eq!(
        root.set_status_rpc_git_info(Some(StatusRpcGitInfo {
            hash: Some("abc123".to_owned()),
            branch: Some("main".to_owned()),
        })),
        None
    );

    assert_eq!(root.status_rpc_current_ledger_index(), Some(778));
    assert_eq!(root.status_rpc_queue_report(), Some(first.clone()));
    assert_eq!(root.status_rpc_peer_count(), Some(12));
    assert_eq!(root.status_rpc_network_id(), Some(21337));
    assert_eq!(root.status_rpc_last_close(), Some(last_close.clone()));
    assert_eq!(root.status_rpc_hostid(), Some("host-one".to_owned()));
    assert_eq!(
        root.status_rpc_server_domain(),
        Some("example.com".to_owned())
    );
    assert_eq!(root.status_rpc_node_size(), Some("huge".to_owned()));
    assert_eq!(root.status_rpc_io_latency_ms(), Some(17));
    assert_eq!(
        root.status_rpc_complete_ledgers(),
        Some("32570-918244".to_owned())
    );
    assert_eq!(root.status_rpc_fetch_pack(), Some(9));
    assert_eq!(
        root.status_rpc_git_info(),
        Some(StatusRpcGitInfo {
            hash: Some("abc123".to_owned()),
            branch: Some("main".to_owned()),
        })
    );

    let snapshot = root.status_rpc_state().snapshot();
    assert_eq!(snapshot.current_ledger_index, Some(778));
    assert_eq!(snapshot.queue_report, Some(first));
    assert_eq!(snapshot.peer_count, Some(12));
    assert_eq!(snapshot.network_id, Some(21337));
    assert_eq!(snapshot.last_close, Some(last_close));
    assert_eq!(snapshot.hostid, Some("host-one".to_owned()));
    assert_eq!(snapshot.server_domain, Some("example.com".to_owned()));
    assert_eq!(snapshot.node_size, Some("huge".to_owned()));
    assert_eq!(snapshot.io_latency_ms, Some(17));
    assert_eq!(snapshot.complete_ledgers, Some("32570-918244".to_owned()));
    assert_eq!(snapshot.fetch_pack, Some(9));
    assert_eq!(
        snapshot.git_info,
        Some(StatusRpcGitInfo {
            hash: Some("abc123".to_owned()),
            branch: Some("main".to_owned()),
        })
    );
}

#[test]
fn application_root_shapes_admin_pubkey_and_validator_list_summary_inputs() {
    let mut root = ApplicationRoot::new(0).expect("root shell should build");

    assert_eq!(root.admin_pubkey_validator(), "none");

    let local_signing_key = protocol::PublicKey::from_bytes([0x02; 33]);
    assert!(
        root.validators()
            .load(Some(local_signing_key), &[], &[], None)
    );
    let local_public_key = root
        .validators()
        .local_public_key()
        .expect("local validator key should be set");
    let validator_public_key = protocol::PublicKey::from_bytes([0x03; 33]);
    assert_eq!(root.set_validation_public_key(validator_public_key), None);
    assert_eq!(root.validation_public_key(), Some(validator_public_key));
    assert_eq!(
        root.admin_pubkey_validator(),
        local_public_key.to_node_public_base58()
    );

    let snapshot = root.validator_list_status_snapshot();
    assert_eq!(snapshot.count, 0);
    assert_eq!(snapshot.validator_list_threshold, 1);
    assert_eq!(snapshot.status, app::ValidatorListStatus::Unknown);
    assert_eq!(snapshot.expiration, app::ValidatorListExpiration::Unknown);
}

#[test]
fn application_root_owns_shared_validator_list_state_for_server_info_reads() {
    let root = ApplicationRoot::new(0).expect("root shell should build");
    let validators = root.validators();

    assert_eq!(validators.quorum(), 1);
    assert_eq!(validators.count(), 0);
    assert_eq!(validators.expires(), None);

    let JsonValue::Object(json) = validators.get_json() else {
        panic!("validator list json should be object");
    };
    assert_eq!(json.get("validation_quorum"), Some(&JsonValue::Unsigned(1)));
    let JsonValue::Object(summary) = json
        .get("validator_list")
        .expect("validator_list summary should exist")
    else {
        panic!("validator_list summary should be object");
    };
    assert_eq!(summary.get("count"), Some(&JsonValue::Unsigned(0)));
    assert_eq!(
        summary.get("status"),
        Some(&JsonValue::String("unknown".to_owned()))
    );
    assert_eq!(
        summary.get("expiration"),
        Some(&JsonValue::String("unknown".to_owned()))
    );
    assert_eq!(
        summary.get("validator_list_threshold"),
        Some(&JsonValue::Unsigned(1))
    );
}

#[test]
fn application_root_exposes_registry_backed_journal_and_wallet_state() {
    let root = ApplicationRoot::new(0).expect("root shell should build");

    assert!(root.get_logs().journal("registry").entries().is_empty());
    assert!(!root.get_config().wallet_db_path.as_os_str().is_empty());
    assert_eq!(root.get_peer_reservations().list().len(), 0);
    assert!(root.status_metrics().is_some());

    let journal = root.get_journal("registry");
    LoadManagerJournal::info(journal.as_ref(), "hello");
    LoadManagerJournal::warn(journal.as_ref(), "world");

    let entries = root.get_logs().journal("registry").entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].message, "hello");
    assert_eq!(entries[1].message, "world");

    let factory = root.load_monitor_journal_factory();
    let monitor = factory.make_load_monitor_journal("monitor");
    monitor.debug("queued");
    monitor.warn("slow");

    let monitor_entries = root.get_logs().journal("monitor").entries();
    assert_eq!(monitor_entries.len(), 2);
    assert_eq!(monitor_entries[0].message, "queued");
    assert_eq!(monitor_entries[1].level, "warn");

    assert_eq!(root.get_app() as *const _, &root as *const _);
    assert!(root.get_wallet_db().get_session().is_autocommit());
    assert!(root.perf_log().snapshot_report().is_object());
}

#[test]
fn application_root_reports_static_validator_list_summary() {
    let root = ApplicationRoot::new(0).expect("root shell should build");
    assert!(root.validators().load(
        None,
        &["n949f75evCHwgyP4fPVgaHqNHxUVN15PsJEZ3B3HnXPcPjcZAoy7".to_owned()],
        &[],
        None,
    ));

    let validators = root.validators();
    assert_eq!(validators.count(), 1);
    assert_eq!(validators.expires(), Some(u32::MAX));

    let JsonValue::Object(json) = validators.get_json() else {
        panic!("validator list json should be object");
    };
    assert_eq!(json.get("validation_quorum"), Some(&JsonValue::Unsigned(1)));
    let JsonValue::Object(summary) = json
        .get("validator_list")
        .expect("validator_list summary should exist")
    else {
        panic!("validator_list summary should be object");
    };
    assert_eq!(summary.get("count"), Some(&JsonValue::Unsigned(1)));
    assert_eq!(
        summary.get("expiration"),
        Some(&JsonValue::String("never".to_owned()))
    );
    assert_eq!(
        summary.get("status"),
        Some(&JsonValue::String("active".to_owned()))
    );
    assert_eq!(
        summary.get("validator_list_threshold"),
        Some(&JsonValue::Unsigned(1))
    );
}

#[test]
fn application_root_can_attach_overlay_status_source_for_server_info_reads() {
    let mut root = ApplicationRoot::new(0).expect("root shell should build");

    assert!(root.overlay_status().is_none());

    let overlay = Arc::new(
        OverlayImpl::new(overlay_setup(Some(21_338)), Arc::new(TestHandoff)).expect("overlay"),
    );
    let first = overlay_peer(1, 11);
    let second = overlay_peer(2, 12);
    overlay.activate(Arc::clone(&first));
    overlay.activate(Arc::clone(&second));
    overlay.inc_jq_trans_overflow();
    overlay.inc_jq_trans_overflow();
    overlay.inc_jq_trans_overflow();
    overlay.inc_peer_disconnect();
    overlay.inc_peer_disconnect();
    overlay.inc_peer_disconnect();
    overlay.inc_peer_disconnect();
    overlay.inc_peer_disconnect_charges();
    overlay.inc_peer_disconnect_charges();
    overlay.inc_peer_disconnect_charges();
    overlay.inc_peer_disconnect_charges();
    overlay.inc_peer_disconnect_charges();

    let overlay_status_source: Arc<dyn OverlayStatusSource> = overlay.clone();
    assert!(root.attach_overlay_status(overlay_status_source).is_none());
    let attached = root
        .overlay_status()
        .expect("overlay status source should be attached");
    assert_eq!(
        attached.status_snapshot(),
        OverlayStatusSnapshot {
            peers: 2,
            network_id: Some(21_338),
            jq_trans_overflow: 3,
            peer_disconnects: 4,
            peer_disconnect_charges: 5,
        }
    );

    overlay.inc_jq_trans_overflow();
    overlay.inc_peer_disconnect();
    overlay.inc_peer_disconnect_charges();

    assert_eq!(
        root.overlay_status()
            .expect("overlay status source should still exist")
            .status_snapshot(),
        OverlayStatusSnapshot {
            peers: 2,
            network_id: Some(21_338),
            jq_trans_overflow: 4,
            peer_disconnects: 5,
            peer_disconnect_charges: 6,
        }
    );
}

#[test]
fn application_root_can_wire_overlay_to_app_owned_membership_sources() {
    let root = ApplicationRoot::new(0).expect("root shell should build");
    let overlay =
        OverlayImpl::new(overlay_setup(Some(21_338)), Arc::new(TestHandoff)).expect("overlay");
    let reserved = overlay_peer(3, 61);
    let clustered = overlay_peer(4, 62);

    root.wire_overlay_membership_sources(&overlay);
    overlay.activate(Arc::clone(&reserved));
    overlay.activate(Arc::clone(&clustered));

    assert!(overlay.reservations().list().is_empty());
    assert!(!reserved.reserved());
    assert!(!clustered.cluster());

    root.peer_reservations()
        .insert_or_assign(PeerReservation::new(
            reserved.node_public(),
            "wallet-backed",
        ));
    assert!(root.shared_cluster().update(
        clustered.node_public(),
        "clustered",
        0,
        std::time::SystemTime::now(),
    ));
    root.wire_overlay_membership_sources(&overlay);
    assert!(reserved.reserved());
    assert!(clustered.cluster());
    assert_eq!(
        overlay.cluster().member(clustered.node_public()),
        Some("clustered".to_owned())
    );

    assert_eq!(
        root.peer_reservations().erase(&reserved.node_public()),
        Some(PeerReservation::new(
            reserved.node_public(),
            "wallet-backed"
        ))
    );
    root.wire_overlay_membership_sources(&overlay);
    assert!(!reserved.reserved());
}

#[test]
fn application_root_can_attach_status_metrics_source_for_server_info_reads() {
    let mut root = ApplicationRoot::new(0).expect("root shell should build");

    let default_source = root
        .status_metrics()
        .expect("default status metrics source should exist");
    assert!(default_source.counters_json().is_object());
    assert!(default_source.current_activities_json().is_object());

    let first = Arc::new(FixedStatusMetricsSource {
        counters: json!({"rpc": {"total": {"started": "1"}}}),
        current_activities: json!({"jobs": [], "methods": []}),
        nodestore: json!({"entries": 5}),
        state_accounting: json!({"full": {"transitions": "1", "duration_us": "10"}}),
        server_state_duration_us: Some("10".to_owned()),
        initial_sync_duration_us: None,
    });
    let second = Arc::new(FixedStatusMetricsSource {
        counters: json!({"rpc": {"total": {"started": "2"}}}),
        current_activities: json!({"jobs": [{"job": "transaction"}], "methods": []}),
        nodestore: json!({"entries": 8}),
        state_accounting: json!({"full": {"transitions": "2", "duration_us": "20"}}),
        server_state_duration_us: Some("20".to_owned()),
        initial_sync_duration_us: Some("5".to_owned()),
    });

    assert!(root.attach_status_metrics(first.clone()).is_some());
    let attached = root
        .status_metrics()
        .expect("status metrics source should be attached");
    assert_eq!(attached.counters_json(), first.counters);
    assert_eq!(attached.current_activities_json(), first.current_activities);
    assert_eq!(attached.nodestore_counts_json(), first.nodestore);
    assert_eq!(attached.state_accounting_json(), first.state_accounting);
    assert_eq!(
        attached.server_state_duration_us(),
        first.server_state_duration_us
    );
    assert_eq!(
        attached.initial_sync_duration_us(),
        first.initial_sync_duration_us
    );

    let replaced = root
        .attach_status_metrics(second.clone())
        .expect("status metrics source should be replaced");
    assert_eq!(replaced.counters_json(), first.counters);
    assert_eq!(
        root.status_metrics()
            .expect("replacement status metrics source should exist")
            .counters_json(),
        second.counters
    );
}

#[test]
fn application_root_accept_ledger_does_not_include_signed_invalid_pending_transactions() {
    let mut root = ApplicationRoot::new(0).expect("root shell should build");
    let runtime = root.attach_default_network_ops_runtime();
    let tx = signed_invalid_payment_tx(0x61);
    let shared = Arc::new(Mutex::new(Transaction::new(Arc::clone(&tx))));

    assert!(runtime.stage_transaction(Arc::clone(&shared), false, false, false));
    assert_eq!(root.network_ops_pending_transaction_count(), Some(1));

    let next_open = root
        .accept_ledger(1, 1_234, 10)
        .expect("ledger accept should complete");

    assert_eq!(next_open, 2);
    assert_eq!(root.closed_ledger_seq(), Some(1));
    assert_eq!(root.network_ops_pending_transaction_count(), Some(0));
    assert!(
        root.closed_ledger()
            .expect("closed ledger should be recorded")
            .tx_map()
            .root()
            .is_empty()
    );
    assert_ne!(
        shared.lock().expect("transaction mutex").get_status(),
        TransStatus::INCLUDED
    );
}

#[test]
fn application_root_accept_ledger_builds_from_closed_parent_view() {
    let root = ApplicationRoot::new(0).expect("root shell should build");
    let mut parent = ledger_with_amendments(1, 1_111, &[Uint256::from_u64(7)], &[]);
    parent.set_accepted(1_111, LEDGER_DEFAULT_TIME_RESOLUTION, true);
    let parent = Arc::new(parent);

    root.on_closed_ledger(Arc::clone(&parent));

    let next_open = root
        .accept_ledger(2, 1_234, 10)
        .expect("ledger accept should complete");
    let closed = root
        .closed_ledger()
        .expect("closed ledger should be recorded");

    assert_eq!(next_open, 3);
    assert_eq!(closed.header().seq, 2);
    assert_eq!(closed.header().parent_hash, parent.header().hash);
}

#[test]
fn application_root_can_attach_server_ports_setup_for_server_info_reads() {
    let mut root = ApplicationRoot::new(0).expect("root shell should build");

    assert!(root.server_ports_setup().is_none());
    assert!(root.published_server_ports().is_none());

    let first = Arc::new(ServerPortsSetup {
        ports: vec![ServerPortSetup {
            name: "port_rpc".to_owned(),
            ip: "127.0.0.1".to_owned(),
            port: 5005,
            limit: 0,
            protocols: vec!["http".to_owned(), "ws".to_owned()],
            user: "rpc".to_owned(),
            password: "secret".to_owned(),
            admin_user: String::new(),
            admin_password: String::new(),
            ssl_key: String::new(),
            ssl_cert: String::new(),
            ssl_chain: String::new(),
            ssl_ciphers: String::new(),
            admin_nets_v4: Vec::new(),
            admin_nets_v6: Vec::new(),
            secure_gateway_nets_v4: Vec::new(),
            secure_gateway_nets_v6: Vec::new(),
            standalone_mode: false,
        }],
        client: Some(ServerPortClientSetup {
            secure: false,
            ip: "127.0.0.1".to_owned(),
            port: 5005,
            user: "rpc".to_owned(),
            password: "secret".to_owned(),
            admin_user: String::new(),
            admin_password: String::new(),
        }),
        overlay: Some(ServerPortOverlaySetup {
            ip: "127.0.0.1".to_owned(),
            port: 51235,
            limit: 0,
            secure: true,
            ssl_key: String::new(),
            ssl_cert: String::new(),
            ssl_chain: String::new(),
            ssl_ciphers: String::new(),
        }),
        grpc: None,
    });
    let second = Arc::new(ServerPortsSetup {
        ports: vec![ServerPortSetup {
            name: "port_admin".to_owned(),
            ip: "127.0.0.1".to_owned(),
            port: 6006,
            limit: 0,
            protocols: vec!["https".to_owned(), "peer".to_owned()],
            user: "admin".to_owned(),
            password: "secret".to_owned(),
            admin_user: "rpc".to_owned(),
            admin_password: "secret".to_owned(),
            ssl_key: "server.key".to_owned(),
            ssl_cert: "server.crt".to_owned(),
            ssl_chain: "server.chain".to_owned(),
            ssl_ciphers: "ECDHE+AESGCM".to_owned(),
            admin_nets_v4: vec!["127.0.0.0/8".parse().expect("admin network should parse")],
            admin_nets_v6: Vec::new(),
            secure_gateway_nets_v4: Vec::new(),
            secure_gateway_nets_v6: Vec::new(),
            standalone_mode: false,
        }],
        client: Some(ServerPortClientSetup {
            secure: true,
            ip: "127.0.0.1".to_owned(),
            port: 6006,
            user: "admin".to_owned(),
            password: "secret".to_owned(),
            admin_user: "rpc".to_owned(),
            admin_password: "secret".to_owned(),
        }),
        overlay: Some(ServerPortOverlaySetup {
            ip: "127.0.0.1".to_owned(),
            port: 6006,
            limit: 0,
            secure: true,
            ssl_key: "server.key".to_owned(),
            ssl_cert: "server.crt".to_owned(),
            ssl_chain: "server.chain".to_owned(),
            ssl_ciphers: "ECDHE+AESGCM".to_owned(),
        }),
        grpc: Some(PublishedGrpcPort {
            ip: "127.0.0.1".to_owned(),
            port: "50051".to_owned(),
        }),
    });

    assert!(root.attach_server_ports_setup(first.clone()).is_none());
    let attached = root
        .server_ports_setup()
        .expect("server port setup should be attached");
    assert_eq!(attached.as_ref(), first.as_ref());
    assert_eq!(
        attached.published_server_ports(),
        vec![PublishedServerPort {
            port: "5005".to_owned(),
            protocols: vec!["http".to_owned(), "ws".to_owned()],
            admin_nets_v4_configured: false,
            admin_nets_v6_configured: false,
            admin_user: None,
            admin_password: None,
        }]
    );
    assert_eq!(attached.published_grpc_port(), None);
    assert_eq!(attached.client.as_ref().expect("client setup").port, 5005);
    assert_eq!(
        attached.overlay.as_ref().expect("overlay setup").port,
        51235
    );
    assert_eq!(attached.fd_required(), 1024);

    assert!(root.attach_server_ports_setup(second.clone()).is_some());
    let replaced = root
        .server_ports_setup()
        .expect("replacement server port setup should exist");
    assert_eq!(replaced.as_ref(), second.as_ref());
    assert_eq!(
        replaced.published_server_ports(),
        vec![PublishedServerPort {
            port: "6006".to_owned(),
            protocols: vec!["https".to_owned(), "peer".to_owned()],
            admin_nets_v4_configured: true,
            admin_nets_v6_configured: false,
            admin_user: Some("rpc".to_owned()),
            admin_password: Some("secret".to_owned()),
        }]
    );
    assert_eq!(
        replaced.published_grpc_port(),
        Some(PublishedGrpcPort {
            ip: "127.0.0.1".to_owned(),
            port: "50051".to_owned(),
        })
    );
    assert_eq!(replaced.fd_required(), 1024);
    assert_eq!(
        root.published_server_ports()
            .expect("published projection should come from setup")
            .published_server_ports(),
        replaced.published_server_ports()
    );
    assert_eq!(
        root.published_server_ports()
            .expect("published projection should come from setup")
            .published_grpc_port(),
        replaced.published_grpc_port()
    );
}

#[test]
fn application_root_status_rpc_state_replaces_and_clears_values_explicitly() {
    let root = ApplicationRoot::new(0).expect("root shell should build");
    let first = sample_status_queue_report(800);
    let second = sample_status_queue_report(801);
    let first_last_close = StatusRpcLastClose {
        proposers: 7,
        converge_time: Duration::from_millis(900),
    };
    let second_last_close = StatusRpcLastClose {
        proposers: 8,
        converge_time: Duration::from_secs(1),
    };
    let first_git_info = StatusRpcGitInfo {
        hash: Some("first".to_owned()),
        branch: Some("feature/status".to_owned()),
    };
    let second_git_info = StatusRpcGitInfo {
        hash: Some("second".to_owned()),
        branch: Some("main".to_owned()),
    };

    root.set_status_rpc_current_ledger_index(Some(800));
    root.set_status_rpc_queue_report(Some(first.clone()));
    root.set_status_rpc_peer_count(Some(21));
    root.set_status_rpc_network_id(Some(1025));
    root.set_status_rpc_last_close(Some(first_last_close.clone()));
    root.set_status_rpc_hostid(Some("host-a".to_owned()));
    root.set_status_rpc_server_domain(Some("a.example".to_owned()));
    root.set_status_rpc_node_size(Some("large".to_owned()));
    root.set_status_rpc_io_latency_ms(Some(11));
    root.set_status_rpc_complete_ledgers(Some("1-10".to_owned()));
    root.set_status_rpc_fetch_pack(Some(2));
    root.set_status_rpc_git_info(Some(first_git_info.clone()));

    assert_eq!(
        root.set_status_rpc_current_ledger_index(Some(801)),
        Some(800)
    );
    assert_eq!(
        root.set_status_rpc_queue_report(Some(second.clone())),
        Some(first)
    );
    assert_eq!(root.set_status_rpc_peer_count(Some(22)), Some(21));
    assert_eq!(root.set_status_rpc_network_id(Some(2048)), Some(1025));
    assert_eq!(
        root.set_status_rpc_last_close(Some(second_last_close.clone())),
        Some(first_last_close)
    );
    assert_eq!(
        root.set_status_rpc_hostid(Some("host-b".to_owned())),
        Some("host-a".to_owned())
    );
    assert_eq!(
        root.set_status_rpc_server_domain(Some("b.example".to_owned())),
        Some("a.example".to_owned())
    );
    assert_eq!(
        root.set_status_rpc_node_size(Some("huge".to_owned())),
        Some("large".to_owned())
    );
    assert_eq!(root.set_status_rpc_io_latency_ms(Some(19)), Some(11));
    assert_eq!(
        root.set_status_rpc_complete_ledgers(Some("3-99".to_owned())),
        Some("1-10".to_owned())
    );
    assert_eq!(root.set_status_rpc_fetch_pack(Some(4)), Some(2));
    assert_eq!(
        root.set_status_rpc_git_info(Some(second_git_info.clone())),
        Some(first_git_info)
    );
    assert_eq!(root.status_rpc_current_ledger_index(), Some(801));
    assert_eq!(root.status_rpc_queue_report(), Some(second.clone()));
    assert_eq!(root.status_rpc_peer_count(), Some(22));
    assert_eq!(root.status_rpc_network_id(), Some(2048));
    assert_eq!(
        root.status_rpc_last_close(),
        Some(second_last_close.clone())
    );
    assert_eq!(root.status_rpc_hostid(), Some("host-b".to_owned()));
    assert_eq!(
        root.status_rpc_server_domain(),
        Some("b.example".to_owned())
    );
    assert_eq!(root.status_rpc_node_size(), Some("huge".to_owned()));
    assert_eq!(root.status_rpc_io_latency_ms(), Some(19));
    assert_eq!(root.status_rpc_complete_ledgers(), Some("3-99".to_owned()));
    assert_eq!(root.status_rpc_fetch_pack(), Some(4));
    assert_eq!(root.status_rpc_git_info(), Some(second_git_info.clone()));

    assert_eq!(root.set_status_rpc_current_ledger_index(None), Some(801));
    assert_eq!(root.set_status_rpc_queue_report(None), Some(second.clone()));
    assert_eq!(root.set_status_rpc_peer_count(None), Some(22));
    assert_eq!(root.set_status_rpc_network_id(None), Some(2048));
    assert_eq!(
        root.set_status_rpc_last_close(None),
        Some(second_last_close.clone())
    );
    assert_eq!(root.set_status_rpc_hostid(None), Some("host-b".to_owned()));
    assert_eq!(
        root.set_status_rpc_server_domain(None),
        Some("b.example".to_owned())
    );
    assert_eq!(root.set_status_rpc_node_size(None), Some("huge".to_owned()));
    assert_eq!(root.set_status_rpc_io_latency_ms(None), Some(19));
    assert_eq!(
        root.set_status_rpc_complete_ledgers(None),
        Some("3-99".to_owned())
    );
    assert_eq!(root.set_status_rpc_fetch_pack(None), Some(4));
    assert_eq!(
        root.set_status_rpc_git_info(None),
        Some(second_git_info.clone())
    );
    assert_eq!(root.status_rpc_current_ledger_index(), None);
    assert_eq!(root.status_rpc_queue_report(), None);
    assert_eq!(root.status_rpc_peer_count(), None);
    assert_eq!(root.status_rpc_network_id(), None);
    assert_eq!(root.status_rpc_last_close(), None);
    assert_eq!(root.status_rpc_hostid(), None);
    assert_eq!(root.status_rpc_server_domain(), None);
    assert_eq!(root.status_rpc_node_size(), None);
    assert_eq!(root.status_rpc_io_latency_ms(), None);
    assert_eq!(root.status_rpc_complete_ledgers(), None);
    assert_eq!(root.status_rpc_fetch_pack(), None);
    assert_eq!(root.status_rpc_git_info(), None);
}

#[test]
fn application_root_status_rpc_state_stays_explicitly_decoupled_from_validated_ledger() {
    let root = ApplicationRoot::new(0).expect("root shell should build");

    root.on_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_156, 100, false,
    )));

    assert_eq!(root.validated_ledger_seq(), Some(1_156));
    assert_eq!(
        root.status_rpc_current_ledger_index(),
        None,
        "status RPC index should stay caller-owned until explicitly set"
    );
    assert_eq!(
        root.status_rpc_queue_report(),
        None,
        "status RPC queue report should stay absent until explicitly set"
    );
    assert_eq!(
        root.status_rpc_peer_count(),
        None,
        "peer count should stay caller-owned until explicitly set"
    );
    assert_eq!(
        root.status_rpc_network_id(),
        None,
        "network id should stay caller-owned until explicitly set"
    );
    assert_eq!(
        root.status_rpc_last_close(),
        None,
        "last_close should stay caller-owned until explicitly set"
    );
    assert_eq!(
        root.status_rpc_hostid(),
        None,
        "hostid should stay caller-owned until explicitly set"
    );
    assert_eq!(
        root.status_rpc_server_domain(),
        None,
        "server domain should stay caller-owned until explicitly set"
    );
    assert_eq!(
        root.status_rpc_node_size(),
        None,
        "node size should stay caller-owned until explicitly set"
    );
    assert_eq!(
        root.status_rpc_io_latency_ms(),
        None,
        "io latency should stay caller-owned until explicitly set"
    );
    assert_eq!(
        root.status_rpc_complete_ledgers(),
        None,
        "complete_ledgers should stay caller-owned until explicitly set"
    );
    assert_eq!(
        root.status_rpc_fetch_pack(),
        None,
        "fetch_pack should stay caller-owned until explicitly set"
    );
    assert_eq!(
        root.status_rpc_git_info(),
        None,
        "git info should stay caller-owned until explicitly set"
    );
}

#[test]
fn application_root_exposes_close_time_runtime_fields_for_server_info() {
    let root = ApplicationRoot::new(0).expect("root shell should build");

    assert_eq!(root.close_time_offset_seconds(), 0);

    let adjusted = root
        .time_keeper()
        .adjust_close_time(time::Duration::seconds(240));
    assert_eq!(adjusted.whole_seconds(), root.close_time_offset_seconds());
    assert_eq!(
        root.current_close_time_seconds(),
        root.time_keeper().close_time().as_seconds()
    );
}

#[test]
fn application_root_tracks_unsupported_majority_warning_without_details() {
    let root = ApplicationRoot::new(0).expect("root shell should build");

    assert!(!root.unsupported_majority_warned());
    assert_eq!(root.unsupported_majority_warning_details(), None);

    assert!(!root.set_unsupported_majority_warned(true));
    assert!(root.unsupported_majority_warned());
    assert_eq!(root.unsupported_majority_warning_details(), None);

    assert!(root.set_unsupported_majority_warned(false));
    assert!(!root.unsupported_majority_warned());
    assert_eq!(root.unsupported_majority_warning_details(), None);
}

#[test]
fn application_root_warns_on_validated_ledger_with_unknown_majority() {
    let root = ApplicationRoot::new(0).expect("root shell should build");
    let unsupported = Uint256::from_u64(0xDEAD);
    let ledger = Arc::new(ledger_with_amendments(
        512,
        1_500,
        &[],
        &[(unsupported, 1_200)],
    ));

    assert!(root.on_validated_ledger(Arc::clone(&ledger)));
    assert!(!root.amendment_blocked());
    assert!(root.unsupported_majority_warned());

    let details = root
        .unsupported_majority_warning_details()
        .expect("warning details should be present");
    assert_eq!(details.expected_date, 1_200 + 14 * 24 * 60 * 60);
    assert_eq!(details.expected_date_utc, "2000-Jan-15 00:20:00 UTC");
}

#[test]
fn application_root_blocks_on_validated_ledger_with_unknown_enabled_amendment() {
    let root = ApplicationRoot::new(0).expect("root shell should build");
    let unsupported = Uint256::from_u64(0xBEEF);
    let ledger = Arc::new(ledger_with_amendments(513, 1_600, &[unsupported], &[]));

    assert!(root.on_validated_ledger(Arc::clone(&ledger)));
    assert!(root.amendment_blocked());
    assert!(root.amendment_status().has_unsupported_enabled());
}

#[test]
fn application_root_owns_wallet_db_peer_reservations_and_log_journals() {
    let root = ApplicationRoot::new(0).expect("root shell should build");
    assert!(
        root.load_peer_reservations()
            .expect("peer reservations load")
    );
    let wallet_db = root.wallet_db();

    let connection = wallet_db.get_session();
    let exists: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='PeerReservations'",
            [],
            |row| row.get(0),
        )
        .expect("peer reservations table");
    assert_eq!(exists, 1);
    drop(connection);

    let peer = PublicKey::from_bytes([0x02; 33]);
    root.peer_reservations()
        .insert_or_assign(PeerReservation::new(peer, "primary"));
    let connection = wallet_db.get_session();
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM PeerReservations WHERE PublicKey = ?1",
            [peer.to_node_public_base58()],
            |row| row.get(0),
        )
        .expect("peer reservation row count");
    assert_eq!(count, 1);

    let journal = root.logs().journal("load_manager");
    root.load_manager().start();
    root.load_manager().stop();
    let entries = journal.entries();
    assert!(entries.iter().any(|entry| entry.message == "Starting"));
    assert!(entries.iter().any(|entry| entry.message == "Stopping"));
}

#[test]
fn application_root_mode_owner_uses_live_validated_ledger_age() {
    let root = ApplicationRoot::with_options(ApplicationRootOptions::default()).expect("root");
    let owner = root.network_ops_mode_owner();

    assert_eq!(
        owner.set_operating_mode(NetworkOpsOperatingMode::Connected),
        NetworkOpsOperatingMode::Disconnected
    );
    assert_eq!(owner.operating_mode(), NetworkOpsOperatingMode::Connected);

    assert!(
        root.on_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
            1,
            946_684_800,
            false
        )))
    );
    assert_eq!(
        owner.set_operating_mode(NetworkOpsOperatingMode::Connected),
        NetworkOpsOperatingMode::Connected
    );
    assert_eq!(owner.operating_mode(), NetworkOpsOperatingMode::Syncing);
}
