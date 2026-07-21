// Phase 5: Legacy catchup loop removed; NetworkOpsStrand handles all duties.

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

// Configure jemalloc to immediately return freed pages to the OS.
// Without this, jemalloc retains freed pages as "dirty" for potential reuse,
// so RSS never decreases even after freeing 7.5M tree nodes (33GB+).
// dirty_decay_ms:0 = purge dirty pages immediately
// muzzy_decay_ms:0 = purge muzzy pages immediately
#[cfg(not(target_env = "msvc"))]
#[used]
#[allow(non_upper_case_globals)]
#[unsafe(no_mangle)]
pub static _rjem_malloc_conf: Option<&'static libc::c_char> = Some(unsafe {
    &*b"dirty_decay_ms:0,muzzy_decay_ms:0\0".as_ptr().cast::<libc::c_char>()
});

use app::{
    AppBootstrapOptions, AppBootstrapRuntime, MainRuntime, ManagedComponent,
    build_bootstrap_root, load_basic_config_file,
    parse_bootstrap_args, run_bootstrap_runtime,
};
use basics::base_uint::Uint256;
use basics::basic_config::BasicConfig;
use overlay::Overlay;
use overlay::Peer as _;
// Import PeerSet trait for method access on SimplePeerSet
use rpc::rpc_cmd_to_json;
use server::{ServerRuntime, ServerRuntimeBuildReport};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const PEERFINDER_LIVE_CACHE_TTL: Duration = Duration::from_secs(30);
const PEERFINDER_RECENT_ATTEMPT_DURATION: Duration = Duration::from_secs(60);
const PEERFINDER_SECONDS_PER_MESSAGE: Duration = Duration::from_secs(151);
const PEERFINDER_SECONDS_PER_CONNECT: Duration = Duration::from_secs(10);
const PEERFINDER_MAX_HOPS: u32 = 6;
const PEERFINDER_NUMBER_OF_ENDPOINTS: usize = (2 * PEERFINDER_MAX_HOPS) as usize;
const PEERFINDER_MAX_CONNECT_ATTEMPTS: usize = 20;
const PEERFINDER_MAX_REDIRECTS: usize = 30;
const PEERFINDER_OUT_PERCENT: usize = 15;
const PEERFINDER_MIN_OUTBOUND: usize = 10;
const PEERFINDER_BOOTCACHE_SIZE: usize = 1000;
const PEERFINDER_BOOTCACHE_PRUNE_PERCENT: usize = 10;
const PEERFINDER_BOOTCACHE_UPDATE_COOLDOWN: Duration = Duration::from_secs(60);
const MAX_DIRECT_LEDGER_DATA_PER_TICK: usize = 4096;

fn full_sync_debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("XRPLD_FULL_SYNC_DEBUG")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    })
}


fn debug_hash8(hash: &Uint256) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}",
        hash.data()[0],
        hash.data()[1],
        hash.data()[2],
        hash.data()[3]
    )
}

macro_rules! full_sync_debug {
    ($($arg:tt)*) => {
        if crate::full_sync_debug_enabled() {
            tracing::debug!(target: "full_sync", $($arg)*);
        }
    };
}

#[derive(Debug, Clone, Copy)]
struct KnownEndpoint {
    hops: u32,
    last_seen: Instant,
}

enum PeerfinderBootcacheEvent {
    Redirects(Vec<std::net::SocketAddr>),
    Success(std::net::SocketAddr),
    Failure(std::net::SocketAddr),
}

fn remember_known_endpoint(
    known_endpoints: &mut HashMap<std::net::SocketAddr, KnownEndpoint>,
    endpoint: std::net::SocketAddr,
    hops: u32,
    now: Instant,
) {
    known_endpoints
        .entry(endpoint)
        .and_modify(|known| {
            known.hops = known.hops.min(hops);
            known.last_seen = now;
        })
        .or_insert(KnownEndpoint {
            hops,
            last_seen: now,
        });
}

fn prune_known_endpoints(
    known_endpoints: &mut HashMap<std::net::SocketAddr, KnownEndpoint>,
    now: Instant,
) {
    known_endpoints.retain(|_, endpoint| {
        now.saturating_duration_since(endpoint.last_seen) <= PEERFINDER_LIVE_CACHE_TTL
    });
}

fn prune_recent_connect_attempts(
    recent_attempts: &mut HashMap<std::net::IpAddr, Instant>,
    now: Instant,
) {
    recent_attempts.retain(|_, until| *until > now);
}

/// Shared PeerFinder bookkeeping state, matching reference PeerFinder::Logic's
/// internal livecache/bootcache/recent-attempts state. Wrapped in a single
/// mutex so both the dedicated overlay-timer thread (sendEndpoints/autoConnect)
/// and anything else that needs to inspect it can share ownership safely,
/// without pinning this state to a specific thread's stack.
struct PeerfinderState {
    known_endpoints: HashMap<std::net::SocketAddr, KnownEndpoint>,
    redirect_bootcache: std::collections::BTreeMap<std::net::SocketAddr, i32>,
    bootcache_dirty: bool,
    recent_autoconnect_attempts: HashMap<std::net::IpAddr, Instant>,
    last_bootcache_save_at: Instant,
}

impl PeerfinderState {
    fn new(peerfinder_bootcache_path: Option<&std::path::Path>) -> Self {
        Self {
            known_endpoints: HashMap::new(),
            redirect_bootcache: peerfinder_bootcache_path
                .map(load_peerfinder_bootcache)
                .unwrap_or_default(),
            bootcache_dirty: false,
            recent_autoconnect_attempts: HashMap::new(),
            last_bootcache_save_at: Instant::now(),
        }
    }
}

/// Dependencies for the dedicated overlay-timer thread. All fields are
/// cheaply cloneable (Arc-backed), so this can be constructed once and moved
/// into the spawned thread.
struct OverlayTimerDeps {
    app: app::ApplicationRoot,
    ledger_data_rx: Arc<Mutex<Option<std::sync::mpsc::Receiver<overlay::PeerMessage<overlay::TmLedgerData>>>>>,
    acq_registry: AcqRegistry,
    shared_fetch_pack: Arc<ledger::FetchPackCache>,
    loaded_ledger_runtime: Option<app::AppLoadedLedgerRuntime>,
    peerfinder_bootcache_path: Option<PathBuf>,
    peerfinder_state: Arc<Mutex<PeerfinderState>>,
    stop: Arc<CatchUpState>,
}

/// Spawn the dedicated overlay-timer thread, matching reference
/// OverlayImpl::Timer's fixed 1-second boost::asio::steady_timer cadence.
///
/// This replaces the previous approach of checking `elapsed() >= 1s` inside
/// the main catchup loop's ~1ms busy-poll body: that coupled a 1Hz duty to a
/// thread spinning at ~1000Hz, which churned the queued_inbound mutex and
/// starved the consensus-driver thread badly enough that peer proposals
/// looked stale by the propose_freshness cutoff, causing nodes to silently
/// diverge. A dedicated thread that sleeps exactly 1 second per iteration —
/// the same pattern already used by `consensus::driver::spawn_heartbeat` —
/// runs these duties at the correct cadence independent of any other loop's
/// polling rate.
///
/// Handles, in order, matching reference OverlayImpl::onTimer:
/// sendEndpoints (PeerFinder::Logic::buildEndpointsForPeers), autoConnect
/// (PeerFinder::Logic::autoconnect), ping (PeerImp::onTimer), and the
/// inbound message-queue duties (ledger_data routing, get_objects routing,
/// get_ledgers serving, and NetworkOPs::processTransaction for peer-relayed
/// transactions).
fn spawn_overlay_timer(deps: OverlayTimerDeps) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("xrpld-overlay-timer".to_owned())
        .spawn(move || {
            tracing::info!(target: "overlay", "Overlay timer thread started (1s)");
            let (bootcache_tx, bootcache_rx) =
                std::sync::mpsc::channel::<PeerfinderBootcacheEvent>();
            let mut last_endpoints_at = Instant::now()
                .checked_sub(PEERFINDER_SECONDS_PER_MESSAGE)
                .unwrap_or_else(Instant::now);
            let mut last_auto_connect_at = Instant::now()
                .checked_sub(PEERFINDER_SECONDS_PER_CONNECT)
                .unwrap_or_else(Instant::now);

            while !deps.stop.stop.load(Ordering::Acquire) {
                thread::sleep(Duration::from_secs(1));
                if deps.stop.stop.load(Ordering::Acquire) {
                    break;
                }

                let Some(overlay_runtime) = deps.app.overlay_runtime() else {
                    continue;
                };
                use overlay::Overlay as _;
                let peers = overlay_runtime.overlay().active_peers();

                // sendEndpoints (reference PeerFinder::Logic::buildEndpointsForPeers)
                if last_endpoints_at.elapsed() >= PEERFINDER_SECONDS_PER_MESSAGE {
                    last_endpoints_at = Instant::now();
                    let mut state = deps.peerfinder_state.lock().expect("peerfinder state");
                    prune_known_endpoints(&mut state.known_endpoints, Instant::now());
                    let listening_port = overlay_runtime.listener_setup().map(|setup| setup.port);
                    for peer in &peers {
                        let endpoints_v2 = build_endpoint_broadcast(
                            listening_port,
                            &state.known_endpoints,
                            peer,
                            Instant::now(),
                        );
                        if endpoints_v2.is_empty() {
                            continue;
                        }
                        let msg = overlay::ProtocolMessage::new(
                            overlay::ProtocolPayload::Endpoints(overlay::TmEndpoints {
                                version: 2,
                                endpoints_v2,
                            }),
                        );
                        peer.send(overlay::Message::new(msg, None));
                    }
                }

                // autoConnect (reference PeerFinder::Logic::autoconnect)
                if last_auto_connect_at.elapsed() >= PEERFINDER_SECONDS_PER_CONNECT {
                    last_auto_connect_at = Instant::now();
                    let now = Instant::now();
                    let mut state = deps.peerfinder_state.lock().expect("peerfinder state");
                    prune_known_endpoints(&mut state.known_endpoints, now);
                    prune_recent_connect_attempts(&mut state.recent_autoconnect_attempts, now);
                    while let Ok(event) = bootcache_rx.try_recv() {
                        match event {
                            PeerfinderBootcacheEvent::Redirects(redirect_peers) => {
                                let mut added = 0usize;
                                for addr in redirect_peers.into_iter().take(PEERFINDER_MAX_REDIRECTS) {
                                    if insert_peerfinder_bootcache(&mut state.redirect_bootcache, addr) {
                                        state.bootcache_dirty = true;
                                        added += 1;
                                    }
                                }
                                if added > 0 {
                                    tracing::debug!(target: "peerfinder", added, total = state.redirect_bootcache.len(), "Redirect bootcache updated");
                                }
                            }
                            PeerfinderBootcacheEvent::Success(addr) => {
                                peerfinder_bootcache_success(&mut state.redirect_bootcache, addr);
                                state.bootcache_dirty = true;
                            }
                            PeerfinderBootcacheEvent::Failure(addr) => {
                                peerfinder_bootcache_failure(&mut state.redirect_bootcache, addr);
                                state.bootcache_dirty = true;
                            }
                        }
                    }
                    if state.bootcache_dirty
                        && state.last_bootcache_save_at.elapsed()
                            >= PEERFINDER_BOOTCACHE_UPDATE_COOLDOWN
                    {
                        if let Some(path) = deps.peerfinder_bootcache_path.as_deref() {
                            save_peerfinder_bootcache(path, &state.redirect_bootcache);
                        }
                        state.bootcache_dirty = false;
                        state.last_bootcache_save_at = Instant::now();
                    }
                    let target_outbound_peers = peerfinder_outbound_target(
                        overlay_runtime.overlay().limit(),
                        overlay_runtime.listener_setup().is_some(),
                    );
                    let active_outbound_peers =
                        overlay_runtime.overlay().active_outbound_peers_count();
                    if peers.len() < target_outbound_peers {
                        tracing::debug!(target: "peerfinder", peers = peers.len(), outbound = active_outbound_peers, target_outbound = target_outbound_peers, pending = overlay_runtime.overlay().pending_outbound_attempts(), known_endpoints = state.known_endpoints.len(), "Peer count below target");
                    }
                    if active_outbound_peers < target_outbound_peers
                        && overlay_runtime.overlay().pending_outbound_attempts() == 0
                    {
                        let mut connected_addrs: std::collections::HashSet<std::net::IpAddr> =
                            peers
                                .iter()
                                .map(|p| peerfinder_canonical_ip(p.remote_address().ip()))
                                .collect();
                        let mut scheduled_attempts = 0usize;
                        let selected = select_autoconnect_endpoints(
                            &connected_addrs,
                            &state.known_endpoints,
                            &state.recent_autoconnect_attempts,
                            now,
                        );
                        let selected = if selected.is_empty() {
                            select_bootcache_endpoints(
                                &connected_addrs,
                                &state.redirect_bootcache,
                                &state.recent_autoconnect_attempts,
                                now,
                            )
                        } else {
                            selected
                        };
                        if !selected.is_empty() {
                            tracing::debug!(target: "peerfinder", selected = selected.len(), active_outbound = active_outbound_peers, target_outbound = target_outbound_peers, known_endpoints = state.known_endpoints.len(), bootcache = state.redirect_bootcache.len(), "Autoconnect selected");
                        }
                        for addr in selected {
                            if active_outbound_peers + scheduled_attempts >= target_outbound_peers {
                                break;
                            }
                            connected_addrs.insert(peerfinder_canonical_ip(addr.ip()));
                            state.recent_autoconnect_attempts.insert(
                                peerfinder_canonical_ip(addr.ip()),
                                now + PEERFINDER_RECENT_ATTEMPT_DURATION,
                            );
                            scheduled_attempts += 1;
                            let overlay = Arc::clone(&overlay_runtime.overlay());
                            let bootcache_tx = bootcache_tx.clone();
                            let _ = std::thread::Builder::new()
                                .name("xrpld-auto-connect".to_owned())
                                .spawn(move || {
                                    let rt = tokio::runtime::Builder::new_current_thread()
                                        .enable_all()
                                        .build();
                                    if let Ok(rt) = rt {
                                        match rt.block_on(overlay.connect(addr)) {
                                            Ok(mut result) => {
                                                tracing::info!(target: "peerfinder", %addr, peer_id = result.peer.id(), "Autoconnect connected");
                                                if let Some(session) = result.session.take() {
                                                    overlay.spawn_peer_session(std::sync::Arc::clone(&result.peer), session);
                                                }
                                                let _ = bootcache_tx.send(PeerfinderBootcacheEvent::Success(addr));
                                            }
                                            Err(overlay::ConnectAttemptError::Redirect(redirect_peers)) => {
                                                tracing::debug!(target: "peerfinder", %addr, redirect_count = redirect_peers.len(), "Autoconnect redirected");
                                                let _ = bootcache_tx.send(PeerfinderBootcacheEvent::Redirects(redirect_peers));
                                            }
                                            Err(error) => {
                                                tracing::debug!(target: "peerfinder", %addr, %error, "Autoconnect failed");
                                                let _ = bootcache_tx.send(PeerfinderBootcacheEvent::Failure(addr));
                                            }
                                        }
                                    }
                                });
                        }
                    }
                }

                // === reference PeerImp::onTimer (every 60 seconds): send ping ===
                // Folded into this 1s timer's own cadence tracking below.
                {
                    static_ping_tick(&overlay_runtime, &peers);
                }

                if peers.is_empty() {
                    continue;
                }

                // === OVERLAY DUTIES (reference OverlayImpl::Timer message-queue duties) ===
                let snapshot = overlay_runtime.overlay().take_queued_inbound_snapshot();
                overlay_runtime.overlay().requeue_validations(snapshot.validations);

                // Accumulate discovered endpoints for auto-connect
                {
                    let mut state = deps.peerfinder_state.lock().expect("peerfinder state");
                    let now = Instant::now();
                    for batch in &snapshot.endpoints {
                        for ep in &batch.endpoints {
                            if insert_peerfinder_bootcache(&mut state.redirect_bootcache, ep.endpoint) {
                                state.bootcache_dirty = true;
                            }
                            remember_known_endpoint(&mut state.known_endpoints, ep.endpoint, ep.hops, now);
                        }
                    }
                }

                // --- Route TmLedgerData to acquisitions ---
                let mut direct_messages = Vec::new();
                let mut direct_channel_capped = false;
                for _ in 0..MAX_DIRECT_LEDGER_DATA_PER_TICK {
                    match deps.ledger_data_rx
                        .lock()
                        .expect("ledger_data_rx lock")
                        .as_ref()
                        .map(|rx| rx.try_recv())
                        .unwrap_or(Err(std::sync::mpsc::TryRecvError::Disconnected))
                    {
                        Ok(msg) => direct_messages.push(msg),
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                    }
                }
                if direct_messages.len() == MAX_DIRECT_LEDGER_DATA_PER_TICK {
                    direct_channel_capped = true;
                }
                let total_ledger_data = snapshot.ledger_data.len() + direct_messages.len();
                let mut routed = 0usize;
                let mut unrouted = 0usize;

                for message in &direct_messages {
                    if let Some(cookie) = message.message.request_cookie {
                        if let Some(target) = peers.iter().find(|p| p.id() == cookie) {
                            let mut fwd = message.message.clone();
                            fwd.request_cookie = None;
                            let reply = overlay::ProtocolMessage::new(overlay::ProtocolPayload::LedgerData(fwd));
                            target.send(overlay::Message::new(reply, None));
                        }
                        continue;
                    }
                    if let Some((hash, packet)) = parse_ledger_data_packet(&message.message) {
                        let hash = *hash.as_uint256();
                        if route_ledger_data_to_acq(&deps.acq_registry, &hash, message.peer_id as u64, packet.clone()) {
                            routed += 1;
                        } else {
                            unrouted += 1;
                            if message.message.r#type == 2
                                && let Some((_, packet)) = parse_ledger_data_packet(&message.message) {
                                    let mut fp_store = SharedFetchPack::new(Arc::clone(&deps.shared_fetch_pack));
                                    let _ = app::stash_stale_packet(&packet, &mut fp_store);
                                }
                        }
                    }
                }

                for message in &snapshot.ledger_data {
                    if let Some(cookie) = message.message.request_cookie {
                        if let Some(target) = peers.iter().find(|p| p.id() == cookie) {
                            let mut fwd = message.message.clone();
                            fwd.request_cookie = None;
                            let reply = overlay::ProtocolMessage::new(overlay::ProtocolPayload::LedgerData(fwd));
                            target.send(overlay::Message::new(reply, None));
                        }
                        continue;
                    }
                    if let Some((hash, packet)) = parse_ledger_data_packet(&message.message) {
                        let hash = *hash.as_uint256();
                        if route_ledger_data_to_acq(&deps.acq_registry, &hash, message.peer_id as u64, packet.clone()) {
                            routed += 1;
                        } else {
                            unrouted += 1;
                            if message.message.r#type == 2
                                && let Some((_, packet)) = parse_ledger_data_packet(&message.message) {
                                    let mut fp_store = SharedFetchPack::new(Arc::clone(&deps.shared_fetch_pack));
                                    let _ = app::stash_stale_packet(&packet, &mut fp_store);
                                }
                        }
                    }
                }

                // --- Route TMGetObjectByHash responses ---
                for message in &snapshot.get_objects {
                    if message.message.query { continue; }
                    let ledger_hash = match message.message.ledger_hash.as_deref().and_then(Uint256::from_slice) {
                        Some(h) => h,
                        None => continue,
                    };
                    let packet_type = match message.message.r#type {
                        3 => ledger::InboundLedgerDataType::TransactionNode,
                        4 => ledger::InboundLedgerDataType::StateNode,
                        6 => {
                            for obj in &message.message.objects {
                                let Some(hash_bytes) = obj.hash.as_deref() else { continue };
                                let Some(hash) = Uint256::from_slice(hash_bytes) else { continue };
                                let Some(data) = obj.data.as_ref() else { continue };
                                deps.shared_fetch_pack.add_fetch_pack(hash, data.clone());
                            }
                            for tx in deps.acq_registry.lock().expect("acq registry").values() {
                                let _ = tx.send(AcqMsg::FetchPackReady);
                            }
                            tracing::debug!(target: "inbound_ledger", objects = message.message.objects.len(), "Fetch-pack ingested");
                            continue;
                        }
                        _ => continue,
                    };
                    let nodes: Vec<_> = message.message.objects.iter()
                        .filter_map(|obj| {
                            let data = obj.data.as_ref()?;
                            Some(ledger::InboundLedgerNodeData::new(obj.node_id.clone(), data.clone()))
                        })
                        .collect();
                    if nodes.is_empty() { continue; }
                    let packet = ledger::InboundLedgerPacket::new(packet_type, nodes);
                    route_ledger_data_to_acq(&deps.acq_registry, &ledger_hash, message.peer_id as u64, packet);
                }

                // --- Serve GetLedger requests (reference processLedgerRequest) ---
                if let Some(loaded_ledger_runtime) = deps.loaded_ledger_runtime.as_ref() {
                    for gl in &snapshot.get_ledgers {
                        if let Some(peer) = peers.iter().find(|p| p.id() == gl.peer_id) {
                            serve_get_ledger(loaded_ledger_runtime, &gl.message, peer.as_ref(), &peers);
                        }
                    }
                }

                if total_ledger_data > 0 || !snapshot.get_objects.is_empty() || !snapshot.get_ledgers.is_empty() {
                    tracing::debug!(target: "overlay", total_ledger_data, direct_channel_capped, routed, unrouted, get_ledgers = snapshot.get_ledgers.len(), get_objects = snapshot.get_objects.len(), "Route summary");
                }
            }
            tracing::info!(target: "overlay", "Overlay timer thread stopped");
        })
        .expect("spawn xrpld-overlay-timer")
}

/// Process a single inbound peer transaction, matching reference
/// PeerImp::checkTransaction (called from a JobQueue JtTransaction worker).
/// Applies the transaction to the open ledger via NetworkOPs::processTransaction.
fn process_inbound_transaction(app: &app::ApplicationRoot, raw_transaction: &[u8]) {
    let mut serial = protocol::SerialIter::new(raw_transaction);
    let st_tx = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        protocol::STTx::from_serial_iter(&mut serial)
    })) {
        Ok(tx) => tx,
        Err(_) => return,
    };
    let st_tx = std::sync::Arc::new(st_tx);
    let mut transaction: app::SharedTransaction = std::sync::Arc::new(std::sync::Mutex::new(
        app::tx_queue::transaction::Transaction::new(std::sync::Arc::clone(&st_tx)),
    ));
    if let Some(network_ops_runtime) = app.network_ops_runtime() {
        let _ = network_ops_runtime.process_transaction(
            &mut transaction,
            false,
            false,
            false,
            || false,
            || {},
        );
        let _ = app.apply_network_ops_pending_to_open_ledger();
    }
}

/// Per-60s ping tick tracked via a thread-local-style static using an atomic
/// timestamp, since the overlay-timer thread owns its own 1s loop and has no
/// access to the main loop's `last_ping_at` local variable.
fn static_ping_tick(overlay_runtime: &Arc<app::runtime::overlay_runtime::AppOverlayRuntime>, peers: &[Arc<dyn overlay::Peer>]) {
    use std::sync::atomic::AtomicU64;
    static LAST_PING_SECS: AtomicU64 = AtomicU64::new(0);
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let last = LAST_PING_SECS.load(Ordering::Relaxed);
    if now_secs.saturating_sub(last) < 60 {
        return;
    }
    LAST_PING_SECS.store(now_secs, Ordering::Relaxed);
    let ping_msg = overlay::ProtocolMessage::new(
        overlay::ProtocolPayload::Ping(overlay::message::wire::TmPing {
            r#type: 0,
            seq: Some(basics::random::rand_int_to(u32::MAX)),
            ping_time: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            ),
            net_time: None,
        }),
    );
    let wire = overlay::Message::new(ping_msg, None);
    for p in peers {
        p.send(wire.clone());
    }
    overlay_runtime.overlay().delete_idle_peers();
}

fn peerfinder_canonical_ip(ip: std::net::IpAddr) -> std::net::IpAddr {
    match ip {
        std::net::IpAddr::V6(ipv6) => ipv6
            .to_ipv4_mapped()
            .map(std::net::IpAddr::V4)
            .unwrap_or(std::net::IpAddr::V6(ipv6)),
        std::net::IpAddr::V4(_) => ip,
    }
}

fn peerfinder_bootcache_path(config: &BasicConfig) -> Option<PathBuf> {
    config
        .legacy("database_path")
        .ok()
        .map(|path| PathBuf::from(path).join("peerfinder.db"))
}

fn prune_peerfinder_bootcache(bootcache: &mut BTreeMap<std::net::SocketAddr, i32>) {
    if bootcache.len() <= PEERFINDER_BOOTCACHE_SIZE {
        return;
    }
    let prune_count = (bootcache.len() * PEERFINDER_BOOTCACHE_PRUNE_PERCENT) / 100;
    let mut by_worst_valence = bootcache
        .iter()
        .map(|(addr, valence)| (*addr, *valence))
        .collect::<Vec<_>>();
    by_worst_valence.sort_by(|(left_addr, left), (right_addr, right)| {
        left.cmp(right).then_with(|| left_addr.cmp(right_addr))
    });
    for (addr, _) in by_worst_valence.into_iter().take(prune_count) {
        bootcache.remove(&addr);
    }
}

fn insert_peerfinder_bootcache(
    bootcache: &mut BTreeMap<std::net::SocketAddr, i32>,
    addr: std::net::SocketAddr,
) -> bool {
    let inserted = bootcache
        .insert(addr, *bootcache.get(&addr).unwrap_or(&0))
        .is_none();
    if inserted {
        prune_peerfinder_bootcache(bootcache);
    }
    inserted
}

fn peerfinder_bootcache_success(
    bootcache: &mut BTreeMap<std::net::SocketAddr, i32>,
    addr: std::net::SocketAddr,
) {
    let valence = bootcache.entry(addr).or_insert(0);
    *valence = (*valence).max(0).saturating_add(1);
    prune_peerfinder_bootcache(bootcache);
}

fn peerfinder_bootcache_failure(
    bootcache: &mut BTreeMap<std::net::SocketAddr, i32>,
    addr: std::net::SocketAddr,
) {
    let valence = bootcache.entry(addr).or_insert(0);
    *valence = (*valence).min(0).saturating_sub(1);
    prune_peerfinder_bootcache(bootcache);
}

fn load_peerfinder_bootcache(path: &Path) -> BTreeMap<std::net::SocketAddr, i32> {
    let mut bootcache = BTreeMap::new();
    match rdb::PeerFinderDb::open(path).and_then(|db| db.load_bootcache()) {
        Ok(entries) => {
            for entry in entries {
                if let Ok(addr) = entry.address.parse::<std::net::SocketAddr>() {
                    bootcache.insert(addr, entry.valence);
                }
            }
            tracing::info!(target: "peerfinder", count = bootcache.len(), path = %path.display(), "Bootcache loaded");
        }
        Err(error) => {
            tracing::debug!(target: "peerfinder", path = %path.display(), %error, "Bootcache load skipped");
        }
    }
    bootcache
}

fn save_peerfinder_bootcache(path: &Path, bootcache: &BTreeMap<std::net::SocketAddr, i32>) {
    let entries = bootcache
        .iter()
        .map(|(addr, valence)| rdb::PeerFinderBootcacheEntry {
            address: addr.to_string(),
            valence: *valence,
        })
        .collect::<Vec<_>>();
    if let Err(error) = rdb::PeerFinderDb::open(path).and_then(|db| db.save_bootcache(&entries)) {
        tracing::warn!(target: "peerfinder", path = %path.display(), %error, "Bootcache save failed");
    }
}

fn build_endpoint_broadcast(
    listening_port: Option<u16>,
    known_endpoints: &HashMap<std::net::SocketAddr, KnownEndpoint>,
    peer: &Arc<dyn overlay::Peer>,
    now: Instant,
) -> Vec<overlay::message::wire::tm_endpoints::TmEndpointv2> {
    let mut endpoints = Vec::with_capacity(PEERFINDER_NUMBER_OF_ENDPOINTS);

    // Match reference sendEndpoints shape more closely:
    // - advertise ourselves once at hops=0 when we want incoming peers,
    // - then hand out a bounded selection from the discovered live cache.
    if let Some(port) = listening_port {
        endpoints.push(overlay::message::wire::tm_endpoints::TmEndpointv2 {
            endpoint: std::net::SocketAddr::new(std::net::Ipv6Addr::UNSPECIFIED.into(), port)
                .to_string(),
            hops: 0,
        });
    }

    let mut discovered = known_endpoints
        .iter()
        .filter_map(|(addr, endpoint)| {
            (endpoint.hops > 0 && endpoint.hops <= PEERFINDER_MAX_HOPS + 1)
                .then_some((*addr, *endpoint))
        })
        .collect::<Vec<_>>();
    discovered.sort_by(|(left_addr, left), (right_addr, right)| {
        left.hops
            .cmp(&right.hops)
            .then_with(|| right.last_seen.cmp(&left.last_seen))
            .then_with(|| left_addr.cmp(right_addr))
    });

    let mut seen_ips = HashSet::new();
    for (addr, endpoint) in discovered {
        if endpoints.len() >= PEERFINDER_NUMBER_OF_ENDPOINTS {
            break;
        }
        if peerfinder_canonical_ip(peer.remote_address().ip()) == peerfinder_canonical_ip(addr.ip())
        {
            continue;
        }
        if peer.should_filter_recent_endpoint(addr, endpoint.hops, now, PEERFINDER_LIVE_CACHE_TTL) {
            continue;
        }
        if !seen_ips.insert(peerfinder_canonical_ip(addr.ip())) {
            continue;
        }
        peer.remember_recent_endpoint(addr, endpoint.hops, now, PEERFINDER_LIVE_CACHE_TTL);
        endpoints.push(overlay::message::wire::tm_endpoints::TmEndpointv2 {
            endpoint: addr.to_string(),
            hops: endpoint.hops,
        });
    }

    endpoints
}

fn select_autoconnect_endpoints(
    connected_ips: &std::collections::HashSet<std::net::IpAddr>,
    known_endpoints: &HashMap<std::net::SocketAddr, KnownEndpoint>,
    recent_attempts: &HashMap<std::net::IpAddr, Instant>,
    now: Instant,
) -> Vec<std::net::SocketAddr> {
    let mut candidates = known_endpoints
        .iter()
        .filter_map(|(addr, endpoint)| {
            (endpoint.hops <= PEERFINDER_MAX_HOPS
                && recent_attempts
                    .get(&peerfinder_canonical_ip(addr.ip()))
                    .is_none_or(|until| *until <= now))
            .then_some((*addr, *endpoint))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|(left_addr, left), (right_addr, right)| {
        left.hops
            .cmp(&right.hops)
            .then_with(|| right.last_seen.cmp(&left.last_seen))
            .then_with(|| left_addr.cmp(right_addr))
    });

    let mut seen_ips = connected_ips.clone();
    let mut selected = Vec::new();
    for (addr, _) in candidates {
        if seen_ips.insert(peerfinder_canonical_ip(addr.ip())) {
            selected.push(addr);
        }
        if selected.len() >= PEERFINDER_MAX_CONNECT_ATTEMPTS {
            break;
        }
    }
    selected
}

fn select_bootcache_endpoints(
    connected_ips: &std::collections::HashSet<std::net::IpAddr>,
    bootcache: &BTreeMap<std::net::SocketAddr, i32>,
    recent_attempts: &HashMap<std::net::IpAddr, Instant>,
    now: Instant,
) -> Vec<std::net::SocketAddr> {
    let mut candidates = bootcache
        .iter()
        .map(|(addr, valence)| (*addr, *valence))
        .collect::<Vec<_>>();
    candidates.sort_by(|(left_addr, left), (right_addr, right)| {
        right
            .cmp(left)
            .then_with(|| left_addr.ip().cmp(&right_addr.ip()))
            .then_with(|| left_addr.port().cmp(&right_addr.port()))
    });

    let mut seen_ips = connected_ips.clone();
    let mut selected = Vec::new();
    for (addr, _) in candidates {
        if recent_attempts
            .get(&peerfinder_canonical_ip(addr.ip()))
            .is_some_and(|until| *until > now)
        {
            continue;
        }
        if seen_ips.insert(peerfinder_canonical_ip(addr.ip())) {
            selected.push(addr);
        }
        if selected.len() >= PEERFINDER_MAX_CONNECT_ATTEMPTS {
            break;
        }
    }
    selected
}

fn peerfinder_outbound_target(peer_limit: usize, want_incoming: bool) -> usize {
    if peer_limit == 0 {
        return 0;
    }
    if !want_incoming {
        return peer_limit;
    }
    let computed = ((peer_limit * PEERFINDER_OUT_PERCENT) + 50) / 100;
    peer_limit.min(computed.max(PEERFINDER_MIN_OUTBOUND))
}

#[derive(Debug, Clone, Copy)]
struct CatchupResourceProfile {
    run_data_concurrency: usize,
    acq_tree_cache_size: usize,
    acq_tree_cache_age_seconds: i64,
    acq_full_below_size: usize,
    acq_fetch_pack_size: usize,
    write_dedup_size: usize,
    ledger_fetch_limit: usize,
}

impl CatchupResourceProfile {
    fn for_node_size(node_size: Option<&str>) -> Self {
        let node_profile = app::NodeSizeResourceProfile::for_node_size(node_size);
        match node_size.unwrap_or("medium") {
            "tiny" => Self {
                run_data_concurrency: 2,
                acq_tree_cache_size: node_profile.tree_cache_size,
                acq_tree_cache_age_seconds: node_profile.tree_cache_age_seconds,
                acq_full_below_size: 524_288,
                acq_fetch_pack_size: 8_192,
                write_dedup_size: 131_072,
                ledger_fetch_limit: node_profile.ledger_fetch,
            },
            "small" => Self {
                run_data_concurrency: 3,
                acq_tree_cache_size: node_profile.tree_cache_size,
                acq_tree_cache_age_seconds: node_profile.tree_cache_age_seconds,
                acq_full_below_size: 524_288,
                acq_fetch_pack_size: 16_384,
                write_dedup_size: 262_144,
                ledger_fetch_limit: node_profile.ledger_fetch,
            },
            "large" => Self {
                run_data_concurrency: 6,
                acq_tree_cache_size: node_profile.tree_cache_size,
                acq_tree_cache_age_seconds: node_profile.tree_cache_age_seconds,
                acq_full_below_size: 524_288,
                acq_fetch_pack_size: 49_152,
                write_dedup_size: 786_432,
                ledger_fetch_limit: node_profile.ledger_fetch,
            },
            "huge" => Self {
                run_data_concurrency: 8,
                acq_tree_cache_size: node_profile.tree_cache_size,
                acq_tree_cache_age_seconds: node_profile.tree_cache_age_seconds,
                acq_full_below_size: 524_288,
                acq_fetch_pack_size: 65_536,
                write_dedup_size: 1_048_576,
                ledger_fetch_limit: node_profile.ledger_fetch,
            },
            _ => Self {
                run_data_concurrency: 6,
                acq_tree_cache_size: node_profile.tree_cache_size,
                acq_tree_cache_age_seconds: node_profile.tree_cache_age_seconds,
                acq_full_below_size: 524_288,
                acq_fetch_pack_size: 32_768,
                write_dedup_size: 524_288,
                ledger_fetch_limit: node_profile.ledger_fetch,
            },
        }
    }

    fn with_ledger_fetch_limit_override(mut self, ledger_fetch_limit: Option<usize>) -> Self {
        if let Some(ledger_fetch_limit) = ledger_fetch_limit {
            self.ledger_fetch_limit = ledger_fetch_limit;
        }
        self
    }
}

fn bootstrap_acquire_budget_available(
    validated: u32,
    active_count: usize,
    ledger_fetch_limit: usize,
    already_tracked: bool,
) -> bool {
    if already_tracked {
        return true;
    }

    if validated <= 1 {
        // A cold node must finish one full account-state acquisition before
        // run-ahead can help. Starting several full-state ledgers here only
        // splits peer/disk budget and delays the first accepted ledger.
        return active_count == 0;
    }

    active_count < ledger_fetch_limit.max(1)
}

fn ledger_fetch_limit_override(config: &BasicConfig) -> Result<Option<usize>, String> {
    if !config.exists("ledger_acquisition") {
        return Ok(None);
    }

    let Some(limit) = config
        .section("ledger_acquisition")
        .get::<usize>("ledger_fetch_limit")
        .map_err(|_| "Configured ledger_acquisition.ledger_fetch_limit is invalid".to_owned())?
    else {
        return Ok(None);
    };

    if !(xrpld_cli::LEDGER_FETCH_LIMIT_OVERRIDE_MIN..=xrpld_cli::LEDGER_FETCH_LIMIT_OVERRIDE_MAX)
        .contains(&limit)
    {
        return Err(format!(
            "Configured ledger_acquisition.ledger_fetch_limit must be between {} and {}",
            xrpld_cli::LEDGER_FETCH_LIMIT_OVERRIDE_MIN,
            xrpld_cli::LEDGER_FETCH_LIMIT_OVERRIDE_MAX
        ));
    }

    Ok(Some(limit))
}

/// Try to parse CLI subcommands. Returns Some(ExitCode) if a subcommand was
/// handled, None if the node should start normally.
/// Resolve the RPC URL: if user passed --url explicitly, use it.
/// Otherwise, try to parse the config file to find the HTTP admin port.
fn resolve_rpc_url(parsed: &xrpld_cli::Cli) -> String {
    // If user explicitly set --rpc-url (not the default), use it as-is
    if parsed.rpc_url != "http://127.0.0.1:5005" {
        return parsed.rpc_url.clone();
    }

    // Try to find config and extract the RPC port
    let conf_path = parsed.conf.as_deref().unwrap_or_else(|| {
        if std::path::Path::new("xrpld.cfg").exists() {
            "xrpld.cfg"
        } else {
            ""
        }
    });

    if !conf_path.is_empty()
        && let Ok(content) = std::fs::read_to_string(conf_path)
            && let Some(url) = parse_rpc_url_from_config(&content) {
                return url;
            }

    parsed.rpc_url.clone()
}

/// Parse config to find the first port with protocol = http
fn parse_rpc_url_from_config(content: &str) -> Option<String> {
    let mut in_port_section = false;
    let mut port: Option<u16> = None;
    let mut ip: Option<String> = None;
    let mut is_http = false;

    fn rpc_host(ip: Option<&str>) -> &str {
        match ip {
            Some("0.0.0.0") | None => "127.0.0.1",
            Some(h) => h,
        }
    }

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("[port_") {
            // Save previous section if it was HTTP
            if in_port_section && is_http
                && let Some(p) = port {
                    let host = rpc_host(ip.as_deref());
                    return Some(format!("http://{}:{}", host, p));
                }
            in_port_section = true;
            port = None;
            ip = None;
            is_http = false;
        } else if trimmed.starts_with('[') {
            if in_port_section && is_http
                && let Some(p) = port {
                    let host = rpc_host(ip.as_deref());
                    return Some(format!("http://{}:{}", host, p));
                }
            in_port_section = false;
        } else if in_port_section {
            if let Some(val) = trimmed.strip_prefix("port") {
                if let Some(val) = val.trim().strip_prefix('=') {
                    port = val.trim().parse().ok();
                }
            } else if let Some(val) = trimmed.strip_prefix("ip") {
                if let Some(val) = val.trim().strip_prefix('=') {
                    ip = Some(val.trim().to_string());
                }
            } else if let Some(val) = trimmed.strip_prefix("protocol")
                && let Some(val) = val.trim().strip_prefix('=') {
                    is_http = val.trim().contains("http");
                }
        }
    }

    // Check last section
    if in_port_section && is_http
        && let Some(p) = port {
            let host = rpc_host(ip.as_deref());
            return Some(format!("http://{}:{}", host, p));
        }

    None
}

fn try_cli_subcommand() -> Option<ExitCode> {
    use clap::{CommandFactory, Parser, error::ErrorKind};

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        let _ = xrpld_cli::Cli::command().print_help();
        println!();
        return Some(ExitCode::SUCCESS);
    }

    const VALUE_FLAGS: &[&str] = &["--conf", "-c", "--rpc-url"];
    // Known subcommands
    let subcommands = [
        "status",
        "health",
        "peers",
        "sync-status",
        "rpc",
        "ping",
        "server-info",
        "server-state",
        "server-definitions",
        "ledger-closed",
        "ledger-current",
        "ledger-header",
        "fetch-info",
        "get-counts",
        "can-delete",
        "log-rotate",
        "random",
        "validator-info",
        "validator-list-sites",
        "unl-list",
        "consensus-info",
        "tx-reduce-relay",
        "db-stats",
        "log-level",
        "config",
        "doctor",
        "version",
        "validators",
        "amendments",
        "fee",
        "ledger",
        "account",
        "stop",
        "connect",
        "benchmark",
        "validator-keys",
        "cli",
        "help",
        "--help",
        "-h",
        "--version",
        "-V",
    ];

    let parsed = match xrpld_cli::Cli::try_parse() {
        Ok(parsed) => parsed,
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            let _ = err.print();
            return Some(ExitCode::SUCCESS);
        }
        Err(err)
            if args
                .iter()
                .skip(1)
                .any(|arg| subcommands.contains(&arg.as_str())) =>
        {
            let _ = err.print();
            return Some(ExitCode::FAILURE);
        }
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::UnknownArgument | ErrorKind::InvalidSubcommand
            ) =>
        {
            if let Some(command) = first_command_like_arg(&args, VALUE_FLAGS) {
                print_unknown_command(command, &subcommands);
                return Some(ExitCode::FAILURE);
            }
            // No subcommand found — likely node startup flags (--start, --valid, etc.)
            // Fall through to the node startup path where parse_bootstrap_args handles them.
            return None;
        }
        Err(err) => {
            let _ = err.print();
            return Some(ExitCode::FAILURE);
        }
    };
    let url = resolve_rpc_url(&parsed);
    let url = url.as_str();
    let cmd = parsed.command?;

    let ok = match cmd {
        xrpld_cli::Command::Status => xrpld_cli::status::run(url),
        xrpld_cli::Command::Health => {
            if !xrpld_cli::health::run(url) {
                return Some(ExitCode::FAILURE);
            }
            true
        }
        xrpld_cli::Command::Peers => xrpld_cli::peers::run(url),
        xrpld_cli::Command::SyncStatus => xrpld_cli::sync_status::run(url),
        xrpld_cli::Command::Rpc {
            method,
            params,
            raw,
        } => xrpld_cli::rpc_cmd::run(url, &method, params.as_deref(), raw),
        xrpld_cli::Command::Ping => xrpld_cli::rpc_cmd::run_no_params(url, "ping"),
        xrpld_cli::Command::ServerInfo => xrpld_cli::rpc_cmd::run_no_params(url, "server_info"),
        xrpld_cli::Command::ServerState => xrpld_cli::rpc_cmd::run_no_params(url, "server_state"),
        xrpld_cli::Command::ServerDefinitions => {
            xrpld_cli::rpc_cmd::run_no_params(url, "server_definitions")
        }
        xrpld_cli::Command::LedgerClosed => xrpld_cli::rpc_cmd::run_no_params(url, "ledger_closed"),
        xrpld_cli::Command::LedgerCurrent => {
            xrpld_cli::rpc_cmd::run_no_params(url, "ledger_current")
        }
        xrpld_cli::Command::LedgerHeader => xrpld_cli::rpc_cmd::run_no_params(url, "ledger_header"),
        xrpld_cli::Command::FetchInfo => xrpld_cli::rpc_cmd::run_no_params(url, "fetch_info"),
        xrpld_cli::Command::GetCounts => xrpld_cli::rpc_cmd::run_no_params(url, "get_counts"),
        xrpld_cli::Command::CanDelete { value } => {
            xrpld_cli::rpc_cmd::run_can_delete(url, value.as_deref())
        }
        xrpld_cli::Command::LogRotate => xrpld_cli::rpc_cmd::run_logrotate(url),
        xrpld_cli::Command::Random => xrpld_cli::rpc_cmd::run_no_params(url, "random"),
        xrpld_cli::Command::ValidatorInfo => {
            xrpld_cli::rpc_cmd::run_no_params(url, "validator_info")
        }
        xrpld_cli::Command::ValidatorListSites => {
            xrpld_cli::rpc_cmd::run_no_params(url, "validator_list_sites")
        }
        xrpld_cli::Command::UnlList => xrpld_cli::rpc_cmd::run_no_params(url, "unl_list"),
        xrpld_cli::Command::ConsensusInfo => {
            xrpld_cli::rpc_cmd::run_no_params(url, "consensus_info")
        }
        xrpld_cli::Command::TxReduceRelay => {
            xrpld_cli::rpc_cmd::run_no_params(url, "tx_reduce_relay")
        }
        xrpld_cli::Command::DbStats => xrpld_cli::db_stats::run(url, parsed.conf.as_deref()),
        xrpld_cli::Command::LogLevel { level } => xrpld_cli::log_level::run(url, level.as_deref()),
        xrpld_cli::Command::ConfigCheck => {
            xrpld_cli::config_check::run(parsed.conf.as_deref());
            true
        }
        xrpld_cli::Command::Doctor => {
            xrpld_cli::doctor::run(url, parsed.conf.as_deref());
            true
        }
        xrpld_cli::Command::Version => {
            xrpld_cli::version::run();
            true
        }
        xrpld_cli::Command::Validators => xrpld_cli::validators::run(url),
        xrpld_cli::Command::Amendments => xrpld_cli::amendments::run(url),
        xrpld_cli::Command::Fee => xrpld_cli::fee::run(url),
        xrpld_cli::Command::Ledger { seq } => xrpld_cli::ledger_cmd::run(url, seq),
        xrpld_cli::Command::Account { address } => xrpld_cli::account::run(url, &address),
        xrpld_cli::Command::Stop => xrpld_cli::stop::run(url),
        xrpld_cli::Command::Connect { address } => {
            let result = xrpld_cli::rpc_call(url, "connect", serde_json::json!({"ip": address}));
            match result {
                Ok(_) => {
                    println!(
                        "  {} Connect request sent to {}",
                        console::Style::new().green().apply_to("●"),
                        address
                    );
                    true
                }
                Err(e) => {
                    eprintln!("  {} {}", console::Style::new().red().apply_to("●"), e);
                    false
                }
            }
        }
        xrpld_cli::Command::Benchmark => {
            xrpld_cli::benchmark::run();
            true
        }
        xrpld_cli::Command::Cli => {
            xrpld_cli::interactive::run(url);
            true
        }
        xrpld_cli::Command::ValidatorKeys { action } => {
            use xrpld_cli::ValidatorKeysAction;
            match action {
                ValidatorKeysAction::Generate => xrpld_cli::validator_keys::run_generate(),
                ValidatorKeysAction::CreateToken { secret } => {
                    xrpld_cli::validator_keys::run_create_token(secret.as_deref())
                }
                ValidatorKeysAction::Sign { data } => xrpld_cli::validator_keys::run_sign(&data),
                ValidatorKeysAction::Revoke => xrpld_cli::validator_keys::run_revoke(),
                ValidatorKeysAction::Show => xrpld_cli::validator_keys::run_show(),
            }
            true
        }
        xrpld_cli::Command::ExportSnapshot { output } => run_export_snapshot(url, &output),
        xrpld_cli::Command::LoadSnapshot { input } => {
            run_load_snapshot(&input, parsed.conf.as_deref())
        }
    };
    Some(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn first_command_like_arg<'a>(args: &'a [String], value_flags: &[&str]) -> Option<&'a str> {
    let mut index = 1;
    while index < args.len() {
        let arg = args[index].as_str();
        if value_flags.contains(&arg) {
            index += 2;
            continue;
        }
        if arg.starts_with("--conf=") || arg.starts_with("--rpc-url=") {
            index += 1;
            continue;
        }
        if arg.starts_with('-') {
            index += 1;
            continue;
        }
        return Some(arg);
    }
    None
}

fn print_unknown_command(command: &str, subcommands: &[&str]) {
    eprintln!(
        "  {} Unknown command: {command}",
        console::Style::new().red().apply_to("●")
    );

    let suggestions = command_suggestions(command, subcommands);
    if !suggestions.is_empty() {
        eprintln!(
            "    Did you mean {}?",
            suggestions
                .iter()
                .map(|suggestion| format!("`{suggestion}`"))
                .collect::<Vec<_>>()
                .join(" or ")
        );
    }

    eprintln!("    Run `quaxar --help` to see available commands.");
}

fn command_suggestions<'a>(command: &str, subcommands: &'a [&str]) -> Vec<&'a str> {
    let normalized = command.to_ascii_lowercase();
    let singular = normalized.strip_suffix('s').unwrap_or(&normalized);
    let mut suggestions = subcommands
        .iter()
        .copied()
        .filter(|candidate| {
            let candidate = candidate.to_ascii_lowercase();
            candidate.starts_with(singular)
                || candidate.contains(singular)
                || levenshtein_distance(&normalized, &candidate) <= 3
        })
        .take(3)
        .collect::<Vec<_>>();
    suggestions.sort_unstable();
    suggestions.dedup();
    suggestions
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];

    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right.chars().enumerate() {
            let substitution = previous[right_index] + usize::from(left_char != right_char);
            let insertion = current[right_index] + 1;
            let deletion = previous[right_index + 1] + 1;
            current[right_index + 1] = substitution.min(insertion).min(deletion);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right.len()]
}

fn main() -> ExitCode {
    // Initialize structured logging
    // Check for CLI subcommands first (status, health, peers, etc.)
    // If a subcommand is present, run it and exit without starting the node.
    if let Some(exit) = try_cli_subcommand() {
        return exit;
    }

    use tracing_subscriber::prelude::*;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let (filter_layer, reload_handle) = tracing_subscriber::reload::Layer::new(filter);

    let subscriber = tracing_subscriber::registry().with(filter_layer).with(
        tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_thread_ids(true),
    );
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber");

    app::set_log_reload_fn(move |new_filter: &str| {
        let f = tracing_subscriber::EnvFilter::try_new(new_filter)
            .map_err(|e| format!("Invalid filter: {e}"))?;
        reload_handle
            .reload(f)
            .map_err(|e| format!("Reload failed: {e}"))
    });

    tracing::info!(target: "main", version = env!("CARGO_PKG_VERSION"), "QUAXAR starting");

    let start_time = Instant::now();

    let args: Vec<String> = std::env::args().collect();
    let options = match parse_bootstrap_args(args) {
        Ok(options) => options,
        Err(error) => {
            tracing::error!(target: "main", %error, "Fatal error — shutting down");
            return ExitCode::from(1);
        }
    };

    if !options.rpc_parameters.is_empty() {
        return run_rpc_client(options);
    }

    // Server mode
    let config_path = options.config_path.clone();
    tracing::info!(target: "main", config_path = %config_path.display(), "Configuration loaded");

    // Spin up a Tokio runtime wrapper for async contexts needed during build
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = rt.enter();

    let bootstrap = match build_composed_runtime_from_path(config_path, options) {
        Ok(bootstrap) => {
            tracing::info!(target: "main", "Database opened");
            bootstrap
        }
        Err(error) => {
            tracing::error!(target: "main", %error, "Fatal error — shutting down");
            return ExitCode::from(1);
        }
    };

    tracing::info!(target: "main", "Node fully operational");

    match run_bootstrap_runtime(bootstrap) {
        Ok(()) => {
            let uptime_seconds = start_time.elapsed().as_secs();
            tracing::info!(target: "main", uptime_seconds, "Node stopped");
            ExitCode::SUCCESS
        }
        Err(error) => {
            let uptime_seconds = start_time.elapsed().as_secs();
            tracing::error!(target: "main", %error, "Fatal error — shutting down");
            tracing::info!(target: "main", uptime_seconds, "Node stopped");
            ExitCode::from(1)
        }
    }
}

#[derive(Clone)]
struct BoundServerRuntime<D> {
    runtime: ServerRuntime<D>,
    handler: Arc<app::AppServerHandler>,
    app: app::ApplicationRoot,
    catch_up_state: Arc<CatchUpState>,
    node_store_usage_path: Option<PathBuf>,
    peerfinder_bootcache_path: Option<PathBuf>,
    ledger_fetch_limit_override: Option<usize>,
}

fn select_target_seq(
    validated: u32,
    has_shared_range: bool,
    selection_ceiling: u32,
    quorum_target_seq: Option<u32>,
) -> u32 {
    if selection_ceiling <= validated {
        return 0;
    }

    // ledger (from incoming validations) over the shared tip. This ensures we
    // acquire the ledger that has quorum, not the most recent tip which may
    // not have enough validations yet and which fewer peers may have state for.
    if validated <= 1 {
        if let Some(seq) = quorum_target_seq
            && seq > 1
            && seq <= selection_ceiling
        {
            return seq;
        }
        if has_shared_range {
            return selection_ceiling.max(2);
        }
    }

    let next_seq = validated.saturating_add(1).max(2);
    if next_seq > selection_ceiling {
        return 0;
    }

    // When we can get the hash for validated+1 (from skip list or history),
    // walk sequentially — this is the reference findNewLedgersToPublish path.
    // When we can't (e.g. after GapTooLarge, validated+1 is a future ledger
    // with no known hash), use the quorum target which has a live hash from
    // incoming validations. This lets the node keep jumping toward the tip.
    if let Some(seq) = quorum_target_seq
        && seq > next_seq
        && seq <= selection_ceiling
    {
        return seq;
    }

    next_seq
}

fn select_consensus_acquisition_target(
    validated: u32,
    validated_hash_targets: &[(Uint256, u32)],
) -> Option<(Uint256, u32)> {
    let gap = validated_hash_targets
        .iter()
        .map(|(_, seq)| *seq)
        .min()
        .unwrap_or(0)
        .saturating_sub(validated);

    if validated <= 1 {
        // Cold bootstrap needs one current quorum-backed anchor. reference does not
        // spawn an inbound ledger for every observed validation hash; missing
        // consensus ledgers are requested through the preferred-ledger path and
        // deduplicated by hash inside InboundLedgers.
        validated_hash_targets
            .iter()
            .max_by_key(|(_, seq)| *seq)
            .copied()
    } else if gap <= 5 {
        // Near the validated stream, prefer the closest missing ledger so
        // publishing can advance sequentially.
        validated_hash_targets
            .iter()
            .min_by_key(|(_, seq)| *seq)
            .copied()
    } else {
        // Large catchup gaps use the freshest trusted consensus ledger as the
        // bootstrap/reference target.
        validated_hash_targets.last().copied()
    }
}

fn cold_bootstrap_persisted_validated_target(
    validated: u32,
    last_validated_target: Option<(Uint256, u32)>,
) -> Option<(Uint256, u32)> {
    let _ = (validated, last_validated_target);
    None
}

fn hash_for_seq_from_reference_ledger(
    reference_ledger: &ledger::Ledger,
    target_seq: u32,
) -> Option<basics::sha_map_hash::SHAMapHash> {
    if target_seq == 0 || reference_ledger.header().seq < target_seq {
        return None;
    }

    if reference_ledger.header().seq == target_seq {
        return Some(reference_ledger.header().hash);
    }

    reference_ledger
        .hash_of_seq(target_seq, &ledger::NullLedgerJournal)
        .filter(|hash| !hash.is_zero())
}

fn candidate_ledger_for_seq(target_seq: u32) -> u32 {
    target_seq.saturating_add(255) & !255
}

fn candidate_reference_hash_from_reference_ledger(
    reference_ledger: &ledger::Ledger,
    target_seq: u32,
) -> Option<(u32, basics::sha_map_hash::SHAMapHash)> {
    if target_seq == 0 || reference_ledger.header().seq < target_seq {
        return None;
    }

    if hash_for_seq_from_reference_ledger(reference_ledger, target_seq).is_some() {
        return None;
    }

    let candidate_seq = candidate_ledger_for_seq(target_seq);
    if candidate_seq <= target_seq || reference_ledger.header().seq < candidate_seq {
        return None;
    }

    reference_ledger
        .hash_of_seq(candidate_seq, &ledger::NullLedgerJournal)
        .filter(|hash| !hash.is_zero())
        .map(|hash| (candidate_seq, hash))
}

fn promote_current_ledger(
    app: &app::ApplicationRoot,
    peers: &[Arc<dyn overlay::Peer>],
    ledger: std::sync::Arc<ledger::Ledger>,
) {
    if let Some(lm_rt) = app.ledger_master_runtime() {
        // Rust currently has both the app-owned published/current holder and
        // the app-runtime LedgerMaster published holder. reference has one
        // LedgerMaster owner, so keep both Rust holders aligned here whenever
        // the accepted ledger becomes the app's current published ledger.
        lm_rt
            .ledger_master()
            .set_pub_ledger(std::sync::Arc::clone(&ledger));
    }
    app.on_closed_ledger(std::sync::Arc::clone(&ledger));
    app.note_validated_ledger_for_sync(std::sync::Arc::clone(&ledger));
    app.on_published_ledger(std::sync::Arc::clone(&ledger));
    // Only clear need_network_ledger if we have a real validated ledger (not genesis)
    if ledger.header().seq > 1 {
        app.set_need_network_ledger(false);
    } else {
        app.set_need_network_ledger(true);
    }

    let next_seq = ledger.header().seq.saturating_add(1);
    app.set_status_rpc_current_ledger_index(Some(next_seq));
    let base_fee = ledger.fees().base;
    let load_base = app.load_fee_track().load_base();
    let mut fees = app.validations().store().fees_for_ledger(
        *ledger.header().hash.as_uint256(),
        ledger.header().seq,
        load_base,
    );
    if !fees.is_empty() {
        fees.sort();
        let median = fees[fees.len() / 2];
        app.load_fee_track().set_remote_fee(median);
    }
    let parent_hash = *ledger.header().hash.as_uint256();
    let _ = app.open_ledger().modify(|view| {
        *view = app::AppOpenLedgerView::with_parent_hash(next_seq, base_fee, parent_hash);
        true
    });

    if let Some(lm_rt) = app.ledger_master_runtime() {
        let complete = lm_rt.ledger_master().complete_ledgers();
        if !complete.empty() {
            app.set_status_rpc_complete_ledgers(Some(complete.to_string()));
        }
    }

    let hdr = ledger.header();
    let status_msg = overlay::ProtocolMessage::new(overlay::ProtocolPayload::StatusChange(
        overlay::message::wire::TmStatusChange {
            new_status: Some(1),
            new_event: Some(1),
            ledger_seq: Some(hdr.seq),
            ledger_hash: Some(hdr.hash.as_uint256().data().to_vec()),
            ledger_hash_previous: Some(hdr.parent_hash.as_uint256().data().to_vec()),
            network_time: None,
            first_seq: app.ledger_master_runtime().map(|lm| {
                let cl = lm.ledger_master().complete_ledgers();
                cl.first().unwrap_or(0)
            }),
            last_seq: app.ledger_master_runtime().map(|lm| {
                let cl = lm.ledger_master().complete_ledgers();
                cl.last().unwrap_or(0)
            }),
        },
    ));
    let wire = overlay::Message::new(status_msg, None);
    for peer in peers {
        peer.send(wire.clone());
    }
    update_operating_mode_after_accepted_ledger(app, peers, ledger.as_ref());
}

// Kept for compatibility with LedgerMaster publish-gap classification; the
// current Rust runtime does not route this helper through the active bin path.
#[cfg(test)]
const MAX_LEDGER_GAP_TO_PUBLISH_SEQUENTIALLY: u32 = 100;

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LedgerPublishAdvance {
    FirstPublished,
    GapTooLarge,
    Sequential,
    NothingToPublish,
}

#[cfg(test)]
fn classify_publish_advance(valid_seq: u32, published_seq: Option<u32>) -> LedgerPublishAdvance {
    let Some(published_seq) = published_seq else {
        return LedgerPublishAdvance::FirstPublished;
    };
    if valid_seq > published_seq.saturating_add(MAX_LEDGER_GAP_TO_PUBLISH_SEQUENTIALLY) {
        return LedgerPublishAdvance::GapTooLarge;
    }
    if valid_seq <= published_seq {
        return LedgerPublishAdvance::NothingToPublish;
    }
    LedgerPublishAdvance::Sequential
}

#[cfg(test)]
fn should_retry_publish_after_completed_history(
    acquired_seq: u32,
    published_seq: Option<u32>,
    valid_seq: u32,
) -> bool {
    let Some(published_seq) = published_seq else {
        return false;
    };
    acquired_seq > published_seq && acquired_seq <= valid_seq
}

///
/// After inserting acquired ledgers into history, walk pub_seq+1 → val_seq
/// sequentially. For each seq, look up the ledger in history by hash (using
/// the validated ledger's skip list), then build it using the previous ledger
/// as parent. This guarantees the parent is always available before the child
/// is built — exactly how reference processes ledgers.
///
/// Pure acquire-and-trust path was here (try_advance_catchup and
/// try_promote_ledger_with_validations) — deleted: these were remnants of the
/// legacy catchup loop and are not used by the NetworkOpsStrand runtime.

#[cfg(test)]
fn should_attempt_completed_ledger_promotion(
    acquired_seq: u32,
    current_validated_seq: u32,
) -> bool {
    // behind validLedgerSeq_. They are useful history, not promotion candidates.
    acquired_seq > current_validated_seq
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletedLedgerAcceptance {
    HistoricalCached,
    HeldForQuorum,
    ValidatedAccepted,
}

#[cfg(test)]
impl CompletedLedgerAcceptance {
    fn log_label(self) -> &'static str {
        match self {
            Self::HistoricalCached => "historical_cached",
            Self::HeldForQuorum => "held_for_quorum",
            Self::ValidatedAccepted => "validated_accepted",
        }
    }

    fn promotes_validated_ledger(self) -> bool {
        matches!(self, Self::ValidatedAccepted)
    }
}

#[cfg(test)]
fn classify_completed_ledger_acceptance(
    acquired_seq: u32,
    current_validated_seq: u32,
    is_skip_state: bool,
    check_accept_passed: bool,
) -> CompletedLedgerAcceptance {
    let _ = is_skip_state;
    if !should_attempt_completed_ledger_promotion(acquired_seq, current_validated_seq) {
        return CompletedLedgerAcceptance::HistoricalCached;
    }
    if check_accept_passed {
        return CompletedLedgerAcceptance::ValidatedAccepted;
    }
    CompletedLedgerAcceptance::HeldForQuorum
}

fn preferred_closed_ledger_hash_from_hashes(
    peer_hashes: impl IntoIterator<Item = Uint256>,
    our_closed_hash: Uint256,
    count_our_closed: bool,
) -> Option<Uint256> {
    let mut peer_counts = HashMap::<Uint256, u32>::new();
    if count_our_closed {
        peer_counts.insert(our_closed_hash, 1);
    }

    for hash in peer_hashes {
        if !hash.is_zero() {
            *peer_counts.entry(hash).or_insert(0) += 1;
        }
    }

    peer_counts
        .into_iter()
        .max_by(|(hash_a, count_a), (hash_b, count_b)| {
            count_a.cmp(count_b).then_with(|| hash_a.cmp(hash_b))
        })
        .map(|(hash, _)| hash)
}

fn preferred_closed_ledger_hash(
    trusted_preferred: Option<(u32, Uint256)>,
    min_valid_seq: u32,
    peer_hashes: impl IntoIterator<Item = Uint256>,
    our_closed_hash: Uint256,
    prev_closed_hash: Uint256,
    count_our_closed: bool,
) -> Uint256 {
    let preferred = trusted_preferred
        .map(|(seq, hash)| {
            if seq >= min_valid_seq {
                hash
            } else {
                our_closed_hash
            }
        })
        .or_else(|| {
            preferred_closed_ledger_hash_from_hashes(peer_hashes, our_closed_hash, count_our_closed)
        })
        .unwrap_or(our_closed_hash);

    if preferred != our_closed_hash && preferred == prev_closed_hash {
        return our_closed_hash;
    }

    preferred
}

fn peer_prefers_different_closed_ledger(
    app: &app::ApplicationRoot,
    peers: &[Arc<dyn overlay::Peer>],
    accepted_ledger: &ledger::Ledger,
    count_our_closed: bool,
) -> bool {
    let our_closed_hash = *accepted_ledger.header().hash.as_uint256();
    let trusted_preferred = app
        .validations()
        .validations()
        .lock()
        .expect("validations lock should not be poisoned")
        .get_preferred(&app::validated_ledger_from_ledger(
            accepted_ledger,
            &app::NullRclValidationJournal,
        ));

    let preferred = preferred_closed_ledger_hash(
        trusted_preferred,
        app.validated_ledger_seq().unwrap_or(0),
        peers.iter().map(|peer| peer.closed_ledger_hash()),
        our_closed_hash,
        *accepted_ledger.header().parent_hash.as_uint256(),
        count_our_closed,
    );

    preferred != our_closed_hash
}

fn current_ledger_is_fresh(
    now_close_time: u32,
    last_closed_close_time: u32,
    close_time_resolution: u32,
) -> bool {
    now_close_time < last_closed_close_time.saturating_add(close_time_resolution.saturating_mul(2))
}

fn select_post_acquisition_operating_mode(
    current_mode: app::NetworkOpsOperatingMode,
    need_network_ledger: bool,
    ledger_change: bool,
    current_ledger_fresh: bool,
) -> app::NetworkOpsOperatingMode {
    let mut next_mode = current_mode;

    if matches!(
        next_mode,
        app::NetworkOpsOperatingMode::Connected | app::NetworkOpsOperatingMode::Syncing
    ) && !need_network_ledger
        && !ledger_change
    {
        next_mode = app::NetworkOpsOperatingMode::Tracking;
    }

    if matches!(
        next_mode,
        app::NetworkOpsOperatingMode::Connected | app::NetworkOpsOperatingMode::Tracking
    ) && !need_network_ledger
        && !ledger_change
        && current_ledger_fresh
    {
        next_mode = app::NetworkOpsOperatingMode::Full;
    }

    next_mode
}

fn update_operating_mode_after_accepted_ledger(
    app: &app::ApplicationRoot,
    peers: &[Arc<dyn overlay::Peer>],
    accepted_ledger: &ledger::Ledger,
) {
    let current_mode = app.network_ops_operating_mode();
    let ledger_change = peer_prefers_different_closed_ledger(
        app,
        peers,
        accepted_ledger,
        matches!(
            current_mode,
            app::NetworkOpsOperatingMode::Tracking | app::NetworkOpsOperatingMode::Full
        ),
    );

    // rippled: switchLastClosedLedger — when peers/validations prefer a
    // different ledger, JUMP to it. This is the critical recovery path that
    // prevents the node from getting stuck on a stale chain.
    if ledger_change
        && let Some(lm_rt) = app.ledger_master_runtime() {
            let our_closed_hash = *accepted_ledger.header().hash.as_uint256();
            let trusted_preferred = app
                .validations()
                .validations()
                .lock()
                .expect("validations lock")
                .get_preferred(&app::validated_ledger_from_ledger(
                    accepted_ledger,
                    &app::NullRclValidationJournal,
                ));
            let peer_hashes: Vec<Uint256> = peers.iter()
                .map(|p| p.closed_ledger_hash())
                .collect();
            let preferred_hash = preferred_closed_ledger_hash(
                trusted_preferred,
                app.validated_ledger_seq().unwrap_or(0),
                peer_hashes,
                our_closed_hash,
                *accepted_ledger.header().parent_hash.as_uint256(),
                true,
            );

            if preferred_hash != our_closed_hash && !preferred_hash.is_zero() {
                // Try to get the preferred ledger from history or inbound
                let ledger_master = lm_rt.ledger_master();
                if let Some(target) = ledger_master.get_ledger_by_hash(
                    basics::sha_map_hash::SHAMapHash::new(preferred_hash),
                ) {
                    tracing::warn!(target: "consensus",
                        our_seq = accepted_ledger.header().seq,
                        target_seq = target.header().seq,
                        "JUMP: switchLastClosedLedger to peer-preferred ledger"
                    );
                    // Promote: set as valid if quorum is met
                    let target_seq = target.header().seq;
                    let validations = app
                        .validations()
                        .store()
                        .trusted_for_ledger_by_sequence(preferred_hash, target_seq);
                    let val_count = app
                        .validators()
                        .negative_unl_filter_validations(validations)
                        .len();
                    let quorum = app.validators().quorum();
                    if val_count >= quorum {
                        let mut promoted = app.ledger_with_node_fetcher(
                            std::sync::Arc::clone(&target),
                        );
                        {
                            let l = std::sync::Arc::make_mut(&mut promoted);
                            l.set_validated();
                            l.set_full();
                            l.finalize_immutable_no_setup();
                        }
                        ledger_master.ledger_history().insert(
                            std::sync::Arc::clone(&promoted), true,
                        );
                        ledger_master.mark_ledger_complete(target_seq);
                        ledger_master.set_valid_ledger_no_sweep(
                            std::sync::Arc::clone(&promoted), None, None,
                        );
                        app.note_validated_ledger_for_sync(std::sync::Arc::clone(&promoted));
                        app.set_need_network_ledger(false);
                        tracing::info!(target: "consensus",
                            seq = target_seq,
                            validations = val_count,
                            "JUMP: validated ledger promoted (switchLastClosedLedger)"
                        );
                    } else {
                        tracing::debug!(target: "consensus",
                            seq = target_seq,
                            val_count,
                            quorum,
                            "JUMP: preferred ledger acquired but quorum not met yet"
                        );
                    }
                } else {
                    // Don't have it — request acquisition (non-blocking)
                    tracing::info!(target: "consensus",
                        hash = %format!("{:016x}", preferred_hash.data()[0] as u64),
                        "JUMP: requesting preferred ledger acquisition"
                    );
                }
            }
        }

    let next_mode = select_post_acquisition_operating_mode(
        current_mode,
        app.need_network_ledger(),
        ledger_change,
        current_ledger_is_fresh(
            app.current_close_time_seconds(),
            accepted_ledger.header().close_time,
            u32::from(accepted_ledger.header().close_time_resolution),
        ),
    );

    if next_mode != current_mode {
        let _ = app.set_network_ops_operating_mode(next_mode);
    }
}

fn node_store_usage_path(config: &BasicConfig) -> Option<PathBuf> {
    let path = config
        .section("node_db")
        .get::<String>("path")
        .ok()
        .flatten()?;
    Some(PathBuf::from(path))
}

fn path_size_bytes(path: &Path) -> u64 {
    let mut total = 0_u64;
    let mut stack = vec![path.to_path_buf()];

    while let Some(next) = stack.pop() {
        let Ok(metadata) = std::fs::metadata(&next) else {
            continue;
        };

        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
            continue;
        }

        if metadata.is_dir() {
            let Ok(entries) = std::fs::read_dir(&next) else {
                continue;
            };
            stack.extend(entries.filter_map(|entry| entry.ok().map(|entry| entry.path())));
        }
    }

    total
}

/// Implements the reference `LedgerMaster::checkAccept(hash, seq)` path.
/// When enough trusted validations arrive for a ledger hash, acquire it
/// and mark it as validated.
///
/// This ports the `checkAccept(hash, seq)` acquisition trigger. Completed
/// inbound ledgers are then routed through the Rust `checkAccept(ledger)`
/// gate before promotion, matching reference's `canBeCurrent` and quorum checks.
///
/// The RPC acquisition
/// Sends validated ledger (hash, seq) to the catchup loop via a channel,
/// matching reference `LedgerMaster::checkAccept` → `InboundLedgers::acquire`
/// which is non-blocking. Called from inside the validations mutex so it
/// must not block or re-acquire that lock.
/// Elevate the current thread to high scheduling priority.
/// Consensus threads must never be starved by RPC workload — if validators
/// can't emit validations on time, the network stalls. This mirrors rippled
/// where the JobQueue consensus thread runs at elevated priority.
fn set_consensus_thread_priority() {
    #[cfg(unix)]
    {
        // Set highest nice value for non-root (-20 requires root, but we can try)
        unsafe {
            // PRIO_PROCESS = 0, current thread = 0
            libc::setpriority(0, 0, -15);
        }
        // On Linux, also try SCHED_RR (real-time round-robin) with low priority
        #[cfg(target_os = "linux")]
        unsafe {
            let param = libc::sched_param { sched_priority: 10 };
            libc::pthread_setschedparam(libc::pthread_self(), libc::SCHED_RR, &param);
        }
        // On macOS, use QOS_CLASS_USER_INTERACTIVE (highest non-real-time)
        #[cfg(target_os = "macos")]
        unsafe {
            libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0);
        }
        tracing::info!(target: "consensus", "Consensus thread elevated to high priority");
    }
}

// Helper types for persistent ledger acquisition
struct NodeStoreFetcher {
    node_store: app::SHAMapStoreNodeStore,
}

impl shamap::family::SHAMapNodeFetcher for NodeStoreFetcher {
    fn fetch_node_object(
        &self,
        hash: basics::sha_map_hash::SHAMapHash,
        ledger_seq: u32,
    ) -> Option<shamap::node_object::NodeObject> {
        let fetched = match &self.node_store {
            app::SHAMapStoreNodeStore::Single(db) => db.fetch_node_object(
                hash.as_uint256(),
                ledger_seq,
                nodestore::FetchType::Synchronous,
                false,
            ),
            app::SHAMapStoreNodeStore::Rotating(db) => db.fetch_node_object(
                hash.as_uint256(),
                ledger_seq,
                nodestore::FetchType::Synchronous,
                false,
            ),
        }?;
        Some(shamap::node_object::NodeObject::new(
            match fetched.object_type() {
                nodestore::NodeObjectType::AccountNode => {
                    shamap::storage::NodeObjectType::AccountNode
                }
                nodestore::NodeObjectType::TransactionNode => {
                    shamap::storage::NodeObjectType::TransactionNode
                }
                nodestore::NodeObjectType::Ledger => shamap::storage::NodeObjectType::Ledger,
                _ => shamap::storage::NodeObjectType::Unknown,
            },
            fetched.data().to_vec(),
            *fetched.hash(),
        ))
    }
}

struct SharedFetchPack {
    cache: Arc<ledger::FetchPackCache>,
}
impl SharedFetchPack {
    fn new(cache: Arc<ledger::FetchPackCache>) -> Self {
        Self { cache }
    }
}
impl ledger::FetchPackContainer for SharedFetchPack {
    fn get_fetch_pack(&mut self, hash: Uint256) -> Option<Vec<u8>> {
        self.cache.get_fetch_pack(hash)
    }
}
impl ledger::FetchPackStore for SharedFetchPack {
    fn add_fetch_pack(&mut self, hash: Uint256, data: Vec<u8>) {
        self.cache.add_fetch_pack(hash, data);
    }
}

/// Messages sent from the main catchup loop to the acquisition processing thread.
/// Matches reference architecture where gotData queues on the network thread and
/// runData processes on a job thread.
enum AcqMsg {
    /// Raw TmLedgerData packet from a peer (reference gotData)
    LedgerData {
        peer_id: u64,
        packet: ledger::InboundLedgerPacket,
    },
    /// Shared fetch-pack cache was populated; re-check local missing nodes now.
    FetchPackReady,
    /// Update the peer list for sending requests
    Peers(Vec<std::sync::Arc<dyn overlay::Peer>>),
    /// Shutdown
    Stop,
}


/// Shared registry of active acquisition channels, keyed by ledger hash.
/// The overlay direct-channel router thread uses this to route TmLedgerData
/// immediately to the right acquisition thread, bypassing the slow catchup loop.
/// This matches reference where gotLedgerData() dispatches directly from the network thread.
type AcqRegistry = Arc<Mutex<HashMap<Uint256, std::sync::mpsc::Sender<AcqMsg>>>>;

fn route_ledger_data_to_acq(
    registry: &AcqRegistry,
    hash: &Uint256,
    peer_id: u64,
    packet: ledger::InboundLedgerPacket,
) -> bool {
    let guard = registry.lock().expect("acq registry lock");
    if let Some(tx) = guard.get(hash) {
        tx.send(AcqMsg::LedgerData { peer_id, packet }).is_ok()
    } else {
        false
    }
}
fn serve_get_ledger(
    runtime: &app::AppLoadedLedgerRuntime,
    request: &overlay::TmGetLedger,
    peer: &dyn overlay::Peer,
    all_peers: &[std::sync::Arc<dyn overlay::Peer>],
) {
    let ledger = match runtime.resolve_request_ledger(request) {
        Ok(Some(ledger)) => ledger,
        Ok(None) => {
            if request.query_type.is_some()
                && request.request_cookie.is_none()
                && request
                    .ledger_hash
                    .as_deref()
                    .and_then(Uint256::from_slice)
                    .is_some()
                && let Some(relay_peer) = all_peers.iter().find(|p| p.id() != peer.id())
            {
                let mut fwd = request.clone();
                fwd.request_cookie = Some(peer.id() as u64);
                let msg = overlay::ProtocolMessage::new(overlay::ProtocolPayload::GetLedger(fwd));
                relay_peer.send(overlay::Message::new(msg, None));
            }
            return;
        }
        Err(error) => {
            tracing::warn!(target: "overlay", ?error, "GetLedger resolve failed");
            return;
        }
    };

    let nodes = match request.itype {
        0 => runtime.build_base_reply_nodes(ledger.as_ref()),
        1 | 2 => {
            match runtime.build_shamap_reply_nodes(ledger.as_ref(), request, peer.is_high_latency())
            {
                Ok(nodes) => nodes,
                Err(error) => {
                    tracing::warn!(target: "overlay", seq = ledger.header().seq, hash = %ledger.header().hash, itype = request.itype, ?error, "GetLedger node reply failed");
                    return;
                }
            }
        }
        _ => return,
    };

    if !nodes.is_empty() {
        let reply = overlay::ProtocolMessage::new(overlay::ProtocolPayload::LedgerData(
            overlay::TmLedgerData {
                ledger_hash: ledger.header().hash.as_uint256().data().to_vec(),
                ledger_seq: ledger.header().seq,
                r#type: request.itype,
                nodes,
                request_cookie: request.request_cookie.map(|c| c as u32),
                error: None,
            },
        ));
        peer.send(overlay::Message::new(reply, None));
    }
}

fn parse_ledger_data_packet(
    message: &overlay::message::wire::TmLedgerData,
) -> Option<(
    basics::sha_map_hash::SHAMapHash,
    ledger::InboundLedgerPacket,
)> {
    let hash_bytes = &message.ledger_hash;
    let hash = Uint256::from_slice(hash_bytes)?;
    let packet_type = match message.r#type {
        0 => ledger::InboundLedgerDataType::Base,
        1 => ledger::InboundLedgerDataType::TransactionNode,
        2 => ledger::InboundLedgerDataType::StateNode,
        _ => return None,
    };
    let nodes = message
        .nodes
        .iter()
        .map(|n| ledger::InboundLedgerNodeData::new(n.nodeid.clone(), n.nodedata.clone()))
        .collect();
    Some((
        basics::sha_map_hash::SHAMapHash::new(hash),
        ledger::InboundLedgerPacket::new(packet_type, nodes),
    ))
}

impl<D> BoundServerRuntime<D> {
    fn new(
        runtime: ServerRuntime<D>,
        handler: Arc<app::AppServerHandler>,
        app: app::ApplicationRoot,
        node_store_usage_path: Option<PathBuf>,
        peerfinder_bootcache_path: Option<PathBuf>,
        ledger_fetch_limit_override: Option<usize>,
    ) -> Self {
        Self {
            runtime,
            handler,
            app,
            catch_up_state: Arc::new(CatchUpState::default()),
            node_store_usage_path,
            peerfinder_bootcache_path,
            ledger_fetch_limit_override,
        }
    }

    fn start_catch_up_loop(&self) {
        // All ledger acquisition, consensus, acceptance, history backfill,
        // and overlay service duties are now handled by the NetworkOpsStrand
        // spawned during bootstrap. This method is retained as a no-op for
        // API compatibility with BoundServerRuntime's ManagedComponent impl.
        tracing::info!(target: "main", "Ledger catch-up delegated to NetworkOpsStrand (no legacy loop)");
    }

    fn stop_catch_up_loop(&self) {
        tracing::info!(target: "main", "Shutdown signal received");
        self.catch_up_state.stop.store(true, Ordering::Release);
        self.app.job_queue().stop();
    }
}

#[derive(Default)]
struct CatchUpState {
    stop: AtomicBool,
}

impl<D> ManagedComponent for BoundServerRuntime<D>
where
    D: server::RpcDispatcher + Clone + Send + Sync + 'static,
{
    fn start(&self) -> Result<(), String> {
        self.runtime.start()?;
        self.start_catch_up_loop();
        self.handler.mark_started(true);
        Ok(())
    }

    fn stop(&self) {
        self.stop_catch_up_loop();
        self.runtime.stop();
        self.handler.mark_started(false);
    }

    fn fd_required(&self) -> usize {
        self.runtime.fd_required()
    }
}

fn build_composed_runtime_from_path(
    path: impl AsRef<std::path::Path>,
    mut options: AppBootstrapOptions,
) -> Result<AppBootstrapRuntime, String> {
    options.config_path = path.as_ref().to_path_buf();
    let config = load_basic_config_file(&options.config_path)?;
    let node_store_usage_path = node_store_usage_path(&config);
    let peerfinder_bootcache_path = peerfinder_bootcache_path(&config);
    let ledger_fetch_limit_override = ledger_fetch_limit_override(&config)?;
    let bootstrap = build_bootstrap_root(&config, &options)?;
    let mut report = bootstrap.report;
    let mut root = bootstrap.root;

    if let Ok(server_build) = ServerRuntime::from_application_root_with_report(&root) {
        if let Some(overlay_runtime) = root.overlay_runtime() {
            let subscriptions = server_build.runtime.subscriptions();
            let subs_clone = subscriptions.clone();
            root.set_ledger_delta_publisher(move |payload| {
                subs_clone.publish_json(server::StreamKind::LedgerDelta, payload);
            });
            overlay_runtime
                .overlay()
                .set_peer_status_publisher(move |payload| {
                    subscriptions.publish_json(server::StreamKind::PeerStatus, payload);
                });

            // Overlay peer listener: inbound connections are accepted when
            // the overlay has a TLS acceptor configured. The overlay's
            // run_listener is spawned by the overlay runtime itself when
            // a TcpListener is provided via overlay.bind().
            // Production binding happens through the app's overlay_runtime
            // configuration which sets up the peer port from [port_peer].
            tracing::info!(target: "main", "Overlay started — connecting to peers");
        }
        let peer_port = report
            .server_configured_ports
            .iter()
            .find(|p| p.contains("peer"))
            .map(|s| s.as_str())
            .unwrap_or("none");
        let rpc_port = report
            .server_configured_ports
            .iter()
            .find(|p| p.contains("rpc"))
            .map(|s| s.as_str())
            .unwrap_or("none");
        let ws_port = report
            .server_configured_ports
            .iter()
            .find(|p| p.contains("ws"))
            .map(|s| s.as_str())
            .unwrap_or("none");
        tracing::info!(target: "main", peer_port, rpc_port, ws_port, "Ports configured");
        bind_server_runtime_into_root(
            &mut root,
            &mut report,
            server_build,
            node_store_usage_path,
            peerfinder_bootcache_path,
            ledger_fetch_limit_override,
        );
    }

    Ok(AppBootstrapRuntime {
        runtime: Arc::new(MainRuntime::new(root)),
        report,
    })
}

fn bind_server_runtime_into_root<D>(
    root: &mut app::ApplicationRoot,
    report: &mut app::AppBootstrapReport,
    server_build: ServerRuntimeBuildReport<D>,
    node_store_usage_path: Option<PathBuf>,
    peerfinder_bootcache_path: Option<PathBuf>,
    ledger_fetch_limit_override: Option<usize>,
) where
    D: server::RpcDispatcher + Clone + Send + Sync + 'static,
{
    let handler = root.server_handler();
    let app_for_runtime = root.clone();
    let configured_ports: Vec<String> = root
        .server_ports_setup()
        .map(|setup| setup.ports.iter().map(|port| port.name.clone()).collect())
        .unwrap_or_default();
    let deferred_protocols = server_build
        .deferred_protocols
        .iter()
        .map(|protocol| format!("{} on {}", protocol.protocol, protocol.port_name))
        .collect::<Vec<_>>();
    handler.configure(configured_ports.clone(), deferred_protocols.clone());
    let runtime = Arc::new(BoundServerRuntime::new(
        server_build.runtime,
        handler,
        app_for_runtime,
        node_store_usage_path,
        peerfinder_bootcache_path,
        ledger_fetch_limit_override,
    ));
    let _ = root.bind_server(runtime);
    report.has_server_runtime = true;
    report.server_configured_ports = configured_ports;
    report.deferred_protocols = deferred_protocols;
    report.fd_required = root.fd_required();
}

fn run_rpc_client(options: AppBootstrapOptions) -> ExitCode {
    let config = match load_basic_config_file(&options.config_path) {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(target: "main", %error, "Configuration error");
            return ExitCode::from(1);
        }
    };

    // Determine RPC endpoint
    let rpc_ip = options.rpc_ip.unwrap_or_else(|| {
        config
            .legacy("rpc_ip")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "127.0.0.1".to_string())
    });
    let rpc_port = options.rpc_port.unwrap_or_else(|| {
        config
            .legacy("rpc_port")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(5005)
    });

    let request_json = match rpc_cmd_to_json(&options.rpc_parameters, 1) {
        Ok(json) => json,
        Err(status) => {
            tracing::error!(target: "main", message = status.message(), "RPC command error");
            return ExitCode::from(1);
        }
    };

    let client = reqwest::blocking::Client::new();
    let url = format!("http://{}:{}/", rpc_ip, rpc_port);

    let body = serde_json::to_string(&request_json).unwrap();

    match client
        .post(&url)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
    {
        Ok(response) => {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            if status.is_success() {
                println!("{}", text);
                ExitCode::SUCCESS
            } else {
                tracing::error!(target: "main", %status, text, "RPC server error");
                ExitCode::from(1)
            }
        }
        Err(error) => {
            tracing::error!(target: "main", %error, "Failed to connect to RPC server");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
#[path = "main/tests.rs"]
mod tests;

// ─── Snapshot CLI handlers ───────────────────────────────────────────────────

fn run_export_snapshot(url: &str, output: &str) -> bool {
    println!("Requesting snapshot export to {}...", output);

    let request_json = serde_json::json!({
        "method": "export_snapshot",
        "params": [{"output": output}]
    });

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();

    let body = serde_json::to_string(&request_json).unwrap();

    match client
        .post(url)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
    {
        Ok(response) => {
            let text = response.text().unwrap_or_default();
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(status) = json["result"]["status"].as_str()
                    && status == "started" {
                        let seq = json["result"]["ledger_seq"].as_u64().unwrap_or(0);
                        println!("  ✓ Export started (ledger seq: {})", seq);
                        println!("  → Output: {}", output);
                        println!("  → Monitor progress: grep snapshot ~/quaxar.log");
                        return true;
                    }
                if let Some(error) = json["result"]["error_message"].as_str() {
                    eprintln!("  ✗ {}", error);
                    return false;
                }
            }
            eprintln!("{}", text);
            false
        }
        Err(error) => {
            eprintln!("Failed to connect to node at {}: {}", url, error);
            eprintln!("Make sure the node is running and the RPC port is accessible.");
            false
        }
    }
}

fn run_load_snapshot(input: &str, conf: Option<&str>) -> bool {
    use nodestore::{DummyScheduler, Manager, ManagerImp, NullJournal, snapshot::load_snapshot};
    use std::path::Path;

    let config = match load_config_for_snapshot(conf) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error loading config: {e}");
            return false;
        }
    };

    let node_db = match config.section("node_db") {
        s if s.exists("path") => s.clone(),
        _ => {
            eprintln!("Error: [node_db] section with 'path' not found in config");
            return false;
        }
    };

    // Resolve sharded NuDB layout: actual files live in xrpldb.NNNN subdirectories.
    let mut node_db = node_db;
    if let Ok(Some(base_path)) = node_db.get::<String>("path") {
        let writable_path = Path::new(&base_path).join("xrpldb.0000");
        if writable_path.join("nudb.dat").exists() {
            node_db.set("path", writable_path.to_string_lossy().into_owned());
        }
    }

    let manager = ManagerImp::instance();
    let scheduler: Arc<dyn nodestore::Scheduler> = Arc::new(DummyScheduler);
    let journal: Arc<dyn nodestore::NodeStoreJournal> = Arc::new(NullJournal);

    let backend = match manager.make_backend(&node_db, 0, scheduler, journal) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error creating backend: {e}");
            return false;
        }
    };

    if let Err(e) = backend.open(true) {
        eprintln!("Error opening backend: {e}");
        return false;
    }

    let input_path = Path::new(input);
    println!("Loading snapshot from {}...", input_path.display());

    match load_snapshot(backend.as_ref(), input_path) {
        Ok(manifest) => {
            println!(
                "Snapshot loaded successfully: ledger_seq={}, chunks={}",
                manifest.ledger_seq,
                manifest.chunks.len()
            );
            backend.sync();
            let _ = backend.close();
            true
        }
        Err(e) => {
            eprintln!("Snapshot load failed: {e}");
            let _ = backend.close();
            false
        }
    }
}

fn load_config_for_snapshot(conf: Option<&str>) -> Result<BasicConfig, String> {
    let default_path = "/etc/opt/xrpld/xrpld.cfg";
    let config_path = conf.unwrap_or(default_path);
    load_basic_config_file(Path::new(config_path))
}
