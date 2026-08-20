//! App-owned overlay runtime assembly and managed ownership.
//!
//! The reference `Application` owns both the overlay config traversal and the
//! `Overlay` instance itself. This module:
//! - parses the app-owned overlay config surface,
//! - builds a real `OverlayImpl`,
//! - exposes that owner through `ApplicationRoot` and `MainRuntime`.
//! - manages the full inbound server graph.

use crate::{
    AppNetworkOpsModeOwner, ManagedComponent, NetworkOpsOperatingMode, ServerPortOverlaySetup,
    ServerPortsSetup, StatusRpcState,
};
use basics::basic_config::{BasicConfig, Section};
use basics::make_ssl_context::{
    TlsIdentityDer, anonymous_tls_identity_der, authenticated_tls_identity_der,
};
use overlay::{Handoff, Overlay, OverlayHandoff, OverlayImpl, Peer, Setup};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::{IpAddr, SocketAddr, TcpListener as StdTcpListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const CRAWL_OPTION_DISABLED: u32 = 0;
pub const CRAWL_OPTION_OVERLAY: u32 = 1 << 0;
pub const CRAWL_OPTION_SERVER_INFO: u32 = 1 << 1;
pub const CRAWL_OPTION_SERVER_COUNTS: u32 = 1 << 2;
pub const CRAWL_OPTION_UNL: u32 = 1 << 3;

const DEFAULT_REDUCE_RELAY_WAIT: Duration = Duration::from_secs(600);
const PEERFINDER_MAX_CONNECT_ATTEMPTS: usize = 20;
// `OverlayImpl::Timer::onTimer` calls `Logic::autoconnect()` every second.
// This is deliberately distinct from the currently-unused
// `PeerFinder::Tuning::kSecondsPerConnect` declaration in local rippled.
const PEERFINDER_AUTOCONNECT_INTERVAL: Duration = Duration::from_secs(1);
const PEERFINDER_MAX_REDIRECTS: usize = 30;
const PEERFINDER_MAX_HOPS: u32 = 6;
const PEERFINDER_BOOTCACHE_SIZE: usize = 1_000;
const PEERFINDER_BOOTCACHE_PRUNE_PERCENT: usize = 10;
const PEERFINDER_BOOTCACHE_UPDATE_COOLDOWN: Duration = Duration::from_secs(60);
const PEERFINDER_LIVECACHE_TTL: Duration = Duration::from_secs(30);
const PEERFINDER_RECENT_ATTEMPT_DURATION: Duration = Duration::from_secs(60);
// M6-F peerfinder DNS hardening: cache resolved outcomes, bound concurrent
// lookups with a semaphore, and cap a single lookup with a deadline so a slow
// resolver can never stall the one-second autoconnect tick.
const PEERFINDER_DNS_TTL: Duration = Duration::from_secs(60);
const PEERFINDER_DNS_RESOLVE_TIMEOUT: Duration = Duration::from_secs(2);
const PEERFINDER_DNS_FAILURE_COOLDOWN: Duration = Duration::from_secs(30);
const PEERFINDER_DNS_MAX_CONCURRENT: usize = 4;
const BOOTCACHE_STATIC_VALENCE: i32 = 32;
const DEFAULT_PEER_PORT: u16 = 51235;
const FIXED_CONNECTION_BACKOFF_MINUTES: [u64; 10] = [1, 1, 2, 3, 5, 8, 13, 21, 34, 55];
const DEFAULT_BOOTSTRAP_PEER_ENDPOINTS: [&str; 4] = [
    "r.ripple.com:51235",
    "sahyadri.isrdc.in:51235",
    "hubs.xrpkuwait.com:51235",
    "hub.xrpl-commons.org:51235",
];

#[cfg(test)]
fn peerfinder_outbound_target(peer_limit: usize, want_incoming: bool) -> usize {
    Setup {
        peer_limit,
        want_incoming,
        ..Setup::default()
    }
    .peer_limits()
    .outbound_max
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BootcacheEntry {
    valence: i32,
}

/// The non-persistent half of rippled's `Livecache`: endpoint advertisements
/// are retained for `kLiveCacheSecondsToLive`, with a lower hop count replacing
/// a higher hop count and higher-hop duplicates left untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LivecacheEntry {
    hops: u32,
    seen_at: Instant,
}

/// The active PeerFinder cache owner. This is intentionally local to the
/// overlay task: SQLite work receives immutable snapshots via `spawn_blocking`,
/// so neither peer transport nor cache mutation waits on database I/O.
#[derive(Debug)]
struct PeerfinderCaches {
    bootcache: HashMap<SocketAddr, BootcacheEntry>,
    livecache: HashMap<SocketAddr, LivecacheEntry>,
    needs_update: bool,
    when_update: Instant,
}

impl PeerfinderCaches {
    fn loaded(entries: Vec<rdb::PeerFinderBootcacheEntry>, now: Instant) -> Self {
        let mut cache = Self {
            bootcache: HashMap::new(),
            livecache: HashMap::new(),
            // `Bootcache::load` calls `clear`, which flags the cache. The
            // following one-second periodic activity canonicalizes even an
            // unchanged store, exactly as rippled does.
            needs_update: true,
            when_update: now,
        };
        for entry in entries {
            if let Ok(endpoint) = entry.address.parse::<SocketAddr>()
                && !endpoint.ip().is_unspecified()
            {
                cache.bootcache.insert(
                    endpoint,
                    BootcacheEntry {
                        valence: entry.valence,
                    },
                );
            }
        }
        cache.prune_bootcache();
        cache
    }

    fn entries(&self) -> Vec<rdb::PeerFinderBootcacheEntry> {
        let mut entries = self
            .bootcache
            .iter()
            .map(|(endpoint, entry)| rdb::PeerFinderBootcacheEntry {
                address: endpoint.to_string(),
                valence: entry.valence,
            })
            .collect::<Vec<_>>();
        // Storage does not define a read order; a stable snapshot makes the
        // SQLite rewrite and regression tests deterministic without changing
        // PeerFinder's valence-based connection ordering.
        entries.sort_by(|left, right| left.address.cmp(&right.address));
        entries
    }

    fn prune_bootcache(&mut self) {
        if self.bootcache.len() <= PEERFINDER_BOOTCACHE_SIZE {
            return;
        }
        let prune_count = (self.bootcache.len() * PEERFINDER_BOOTCACHE_PRUNE_PERCENT) / 100;
        let mut worst = self
            .bootcache
            .iter()
            .map(|(endpoint, entry)| (*endpoint, entry.valence))
            .collect::<Vec<_>>();
        // `Bootcache`'s bimap is ordered by descending valence and erases
        // backward from end; therefore remove the lowest valences first.
        worst.sort_by(|(left_endpoint, left), (right_endpoint, right)| {
            left.cmp(right)
                .then_with(|| left_endpoint.cmp(right_endpoint))
        });
        for (endpoint, _) in worst.into_iter().take(prune_count) {
            self.bootcache.remove(&endpoint);
        }
    }

    fn insert_bootcache(&mut self, endpoint: SocketAddr) -> bool {
        let inserted = match self.bootcache.entry(endpoint) {
            std::collections::hash_map::Entry::Occupied(_) => false,
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(BootcacheEntry { valence: 0 });
                true
            }
        };
        if inserted {
            self.prune_bootcache();
            self.flag_for_update();
        }
        inserted
    }

    fn insert_static_bootcache(&mut self, endpoint: SocketAddr) -> bool {
        match self.bootcache.get_mut(&endpoint) {
            Some(entry) if entry.valence >= BOOTCACHE_STATIC_VALENCE => false,
            Some(entry) => {
                entry.valence = BOOTCACHE_STATIC_VALENCE;
                self.flag_for_update();
                true
            }
            None => {
                self.bootcache.insert(
                    endpoint,
                    BootcacheEntry {
                        valence: BOOTCACHE_STATIC_VALENCE,
                    },
                );
                self.prune_bootcache();
                self.flag_for_update();
                true
            }
        }
    }

    fn on_success(&mut self, endpoint: SocketAddr) {
        match self.bootcache.entry(endpoint) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(BootcacheEntry { valence: 1 });
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let valence = entry.get().valence;
                entry.get_mut().valence = valence.max(0).saturating_add(1);
            }
        }
        self.prune_bootcache();
        self.flag_for_update();
    }

    fn on_failure(&mut self, endpoint: SocketAddr) {
        match self.bootcache.entry(endpoint) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(BootcacheEntry { valence: -1 });
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let valence = entry.get().valence;
                entry.get_mut().valence = valence.min(0).saturating_sub(1);
            }
        }
        self.prune_bootcache();
        self.flag_for_update();
    }

    fn on_redirects<I>(&mut self, endpoints: I)
    where
        I: IntoIterator<Item = SocketAddr>,
    {
        for endpoint in endpoints.into_iter().take(PEERFINDER_MAX_REDIRECTS) {
            self.insert_bootcache(endpoint);
        }
    }

    fn on_learned_endpoint(&mut self, endpoint: SocketAddr, hops: u32, now: Instant) {
        if hops > PEERFINDER_MAX_HOPS + 1 {
            return;
        }
        match self.livecache.get_mut(&endpoint) {
            Some(existing) if hops > existing.hops => return,
            Some(existing) => {
                existing.hops = hops.min(existing.hops);
                existing.seen_at = now;
            }
            None => {
                self.livecache
                    .insert(endpoint, LivecacheEntry { hops, seen_at: now });
            }
        }
        self.insert_bootcache(endpoint);
    }

    fn expire_livecache(&mut self, now: Instant) {
        self.livecache.retain(|_, entry| {
            now.saturating_duration_since(entry.seen_at) <= PEERFINDER_LIVECACHE_TTL
        });
    }

    fn select_livecache(
        &self,
        connected_ips: &BTreeSet<IpAddr>,
        recent_attempts: &HashMap<IpAddr, Instant>,
        now: Instant,
        max_attempts: usize,
    ) -> Vec<SocketAddr> {
        let mut candidates = self
            .livecache
            .iter()
            .filter_map(|(endpoint, entry)| {
                (!connected_ips.contains(&endpoint.ip())
                    && recent_attempts
                        .get(&endpoint.ip())
                        .is_none_or(|until| *until <= now))
                .then_some((*endpoint, *entry))
            })
            .collect::<Vec<_>>();
        // `Logic::autoconnect` hands out the reverse hop histogram, so the
        // max-hop bucket is tried before nearer buckets within a batch.
        candidates.sort_by(|(left_endpoint, left), (right_endpoint, right)| {
            right
                .hops
                .cmp(&left.hops)
                .then_with(|| left_endpoint.cmp(right_endpoint))
        });

        let mut selected = Vec::new();
        let mut seen_ips = connected_ips.clone();
        for (endpoint, _) in candidates {
            if seen_ips.insert(endpoint.ip()) {
                selected.push(endpoint);
            }
            if selected.len() >= max_attempts {
                break;
            }
        }
        selected
    }

    fn flag_for_update(&mut self) {
        self.needs_update = true;
    }

    fn take_update_if_due(
        &mut self,
        now: Instant,
        force: bool,
    ) -> Option<Vec<rdb::PeerFinderBootcacheEntry>> {
        // rippled: whenUpdate_ < now (strict; update only when deadline is
        // strictly in the past). Block when when_update >= now.
        if !self.needs_update || (!force && self.when_update >= now) {
            return None;
        }
        let entries = self.entries();
        self.needs_update = false;
        self.when_update = now + PEERFINDER_BOOTCACHE_UPDATE_COOLDOWN;
        Some(entries)
    }
}

fn bootstrap_needs_bootcache_dial(
    active_outbound_peers: usize,
    target_outbound_peers: usize,
) -> bool {
    active_outbound_peers < target_outbound_peers
}

fn bootstrap_can_dial_bootcache(
    auto_connect: bool,
    active_outbound_peers: usize,
    target_outbound_peers: usize,
) -> bool {
    // rippled PeerFinder::Config: autoConnect = !standalone && !peerPrivate.
    // Fixed peers remain independently retried below; this controls only
    // autonomous livecache/bootcache acquisition.
    auto_connect && bootstrap_needs_bootcache_dial(active_outbound_peers, target_outbound_peers)
}

/// `PeerFinder::Counts::attemptsNeeded` applies one global cap to every
/// outbound stage. It intentionally does not shrink a batch to the number of
/// normal outbound slots still needed: fixed slots are exempt from that target
/// and normal attempts can race to the activation gate.
fn peerfinder_attempt_budget(pending_outbound_attempts: usize) -> usize {
    PEERFINDER_MAX_CONNECT_ATTEMPTS.saturating_sub(pending_outbound_attempts)
}

/// `Logic::autoconnect` returns immediately after a fixed batch, and also
/// waits while the fixed set remains below its target but any outbound attempt
/// is still in progress. This prevents livecache and bootcache dials from
/// jumping ahead of fixed peers.
fn fixed_stage_blocks_automatic_dials(
    active_fixed_peers: usize,
    fixed_peer_slots: usize,
    fixed_attempts_started: usize,
    pending_outbound_attempts: usize,
) -> bool {
    active_fixed_peers < fixed_peer_slots
        && (fixed_attempts_started > 0 || pending_outbound_attempts > 0)
}

/// Livecache has the same stage barrier before bootcache fallback: when it
/// supplied a batch, or an earlier batch is in flight, `Logic::autoconnect`
/// returns rather than mixing bootcache addresses into the same cycle.
fn livecache_stage_blocks_bootcache(
    livecache_attempts_started: usize,
    pending_outbound_attempts: usize,
) -> bool {
    livecache_attempts_started > 0 || pending_outbound_attempts > 0
}

fn fixed_retry_delay(failures: usize) -> Duration {
    let index = failures.min(FIXED_CONNECTION_BACKOFF_MINUTES.len().saturating_sub(1));
    Duration::from_secs(FIXED_CONNECTION_BACKOFF_MINUTES[index] * 60)
}

fn fixed_retry_state_or_due(
    state: &HashMap<SocketAddr, (usize, Instant)>,
    address: SocketAddr,
    now: Instant,
) -> (usize, Instant) {
    state.get(&address).copied().unwrap_or((0, now))
}

// Bootcache valence helpers. Exercised by tests today; production call sites
// land with the M6-F peerfinder DNS/retry work.
#[cfg_attr(not(test), allow(dead_code))]
fn remember_bootcache_endpoint(
    bootcache: &mut HashMap<SocketAddr, BootcacheEntry>,
    endpoint: SocketAddr,
    static_entry: bool,
) {
    let desired_valence = if static_entry {
        BOOTCACHE_STATIC_VALENCE
    } else {
        0
    };
    bootcache
        .entry(endpoint)
        .and_modify(|entry| {
            if static_entry {
                entry.valence = entry.valence.max(BOOTCACHE_STATIC_VALENCE);
            }
        })
        .or_insert(BootcacheEntry {
            valence: desired_valence,
        });
}

#[cfg_attr(not(test), allow(dead_code))]
fn remember_bootcache_endpoints<I>(
    bootcache: &mut HashMap<SocketAddr, BootcacheEntry>,
    endpoints: I,
    static_entry: bool,
) where
    I: IntoIterator<Item = SocketAddr>,
{
    for endpoint in endpoints {
        remember_bootcache_endpoint(bootcache, endpoint, static_entry);
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn bootcache_on_success(bootcache: &mut HashMap<SocketAddr, BootcacheEntry>, endpoint: SocketAddr) {
    let entry = bootcache
        .entry(endpoint)
        .or_insert(BootcacheEntry { valence: 0 });
    entry.valence = entry.valence.max(0).saturating_add(1);
}

#[cfg_attr(not(test), allow(dead_code))]
fn bootcache_on_failure(bootcache: &mut HashMap<SocketAddr, BootcacheEntry>, endpoint: SocketAddr) {
    let entry = bootcache
        .entry(endpoint)
        .or_insert(BootcacheEntry { valence: 0 });
    entry.valence = entry.valence.min(0).saturating_sub(1);
}

fn prune_recent_bootcache_attempts(recent_attempts: &mut HashMap<IpAddr, Instant>, now: Instant) {
    recent_attempts.retain(|_, until| *until > now);
}

fn select_bootcache_endpoints(
    connected_ips: &BTreeSet<IpAddr>,
    bootcache: &HashMap<SocketAddr, BootcacheEntry>,
    recent_attempts: &HashMap<IpAddr, Instant>,
    now: Instant,
    max_attempts: usize,
) -> Vec<SocketAddr> {
    let mut ranked = bootcache
        .iter()
        .filter_map(|(endpoint, entry)| {
            if connected_ips.contains(&endpoint.ip()) {
                return None;
            }
            if recent_attempts
                .get(&endpoint.ip())
                .is_some_and(|until| *until > now)
            {
                return None;
            }
            Some((*endpoint, *entry))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|(left_addr, left), (right_addr, right)| {
        right
            .valence
            .cmp(&left.valence)
            .then_with(|| left_addr.cmp(right_addr))
    });

    let mut selected = Vec::new();
    let mut seen_ips = connected_ips.clone();
    for (endpoint, _) in ranked {
        if !seen_ips.insert(endpoint.ip()) {
            continue;
        }
        selected.push(endpoint);
        if selected.len() >= max_attempts {
            break;
        }
    }

    selected
}

#[derive(Debug)]
struct NoServerVerification;

impl ServerCertVerifier for NoServerVerification {
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

#[derive(Debug)]
pub struct BootstrapOverlayHandoff;

impl OverlayHandoff for BootstrapOverlayHandoff {
    fn on_handoff(&self, _request: &http::Request<()>, _remote_address: SocketAddr) -> Handoff {
        Handoff::Accepted
    }
}

pub struct AppOverlayRuntime {
    overlay: Arc<OverlayImpl>,
    listener_setup: Option<ServerPortOverlaySetup>,
    /// A mixed peer+HTTP/WS port is bound by the server runtime exactly once.
    /// The overlay retains its TLS/handshake configuration but must not bind it.
    server_owns_listener: bool,
    fixed_peer_endpoints: Vec<String>,
    bootstrap_peer_endpoints: Vec<String>,
    /// `false` for `[peer_private]`, matching rippled's `autoConnect`.
    bootstrap_can_dial_bootcache: bool,
    peerfinder_bootcache_path: Option<PathBuf>,
    network_ops_mode_owner: Option<AppNetworkOpsModeOwner>,
    status_rpc_state: Option<Arc<StatusRpcState>>,
    listener_task: Mutex<Option<tokio::task::JoinHandle<Result<(), overlay::OverlayError>>>>,
    peerfinder_thread: Mutex<Option<JoinHandle<()>>>,
    started: AtomicBool,
    stopped: AtomicBool,
}

impl std::fmt::Debug for AppOverlayRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppOverlayRuntime")
            .field("network_id", &self.network_id())
            .field("listener_setup", &self.listener_setup)
            .field("server_owns_listener", &self.server_owns_listener)
            .field("fixed_peer_endpoints", &self.fixed_peer_endpoints)
            .field("bootstrap_peer_endpoints", &self.bootstrap_peer_endpoints)
            .field(
                "bootstrap_can_dial_bootcache",
                &self.bootstrap_can_dial_bootcache,
            )
            .field("peerfinder_bootcache_path", &self.peerfinder_bootcache_path)
            .field("started", &self.started())
            .field("stopped", &self.stopped())
            .finish()
    }
}

impl AppOverlayRuntime {
    pub fn new(
        overlay: Arc<OverlayImpl>,
        listener_setup: Option<ServerPortOverlaySetup>,
        server_owns_listener: bool,
        fixed_peer_endpoints: Vec<String>,
        bootstrap_peer_endpoints: Vec<String>,
        bootstrap_can_dial_bootcache: bool,
        peerfinder_bootcache_path: Option<PathBuf>,
        network_ops_mode_owner: Option<AppNetworkOpsModeOwner>,
        status_rpc_state: Option<Arc<StatusRpcState>>,
    ) -> Self {
        Self {
            overlay,
            listener_setup,
            server_owns_listener,
            fixed_peer_endpoints,
            bootstrap_peer_endpoints,
            bootstrap_can_dial_bootcache,
            peerfinder_bootcache_path,
            network_ops_mode_owner,
            status_rpc_state,
            listener_task: Mutex::new(None),
            peerfinder_thread: Mutex::new(None),
            started: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        }
    }

    pub fn overlay(&self) -> Arc<OverlayImpl> {
        Arc::clone(&self.overlay)
    }

    pub fn listener_setup(&self) -> Option<ServerPortOverlaySetup> {
        self.listener_setup.clone()
    }

    pub fn server_owns_listener(&self) -> bool {
        self.server_owns_listener
    }

    pub fn bootstrap_can_dial_bootcache(&self) -> bool {
        self.bootstrap_can_dial_bootcache
    }

    pub fn network_id(&self) -> Option<u32> {
        self.overlay.network_id()
    }

    pub fn has_listener_tls(&self) -> bool {
        self.overlay.has_tls_acceptor()
    }

    pub fn started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }

    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
}

impl ManagedComponent for AppOverlayRuntime {
    fn start(&self) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("overlay runtime has already been stopped".to_owned());
        }

        // Avoid duplicate bootstrap task scheduling if start is called more than once.
        if self.started.swap(true, Ordering::AcqRel) {
            return Ok(());
        }

        tokio::runtime::Handle::try_current().map_err(|_| {
            self.started.store(false, Ordering::Release);
            "overlay runtime requires an active tokio runtime before start".to_owned()
        })?;

        let overlay = Arc::clone(&self.overlay);
        let fixed_endpoints = self.fixed_peer_endpoints.clone();
        let endpoints = self.bootstrap_peer_endpoints.clone();
        let peerfinder_bootcache_path = self.peerfinder_bootcache_path.clone();
        let network_ops_mode_owner = self.network_ops_mode_owner.clone();
        let status_rpc_state = self.status_rpc_state.clone();
        let target_outbound_peers = overlay.peer_limits().outbound_max;
        let auto_connect = self.bootstrap_can_dial_bootcache;
        let peerfinder_thread = std::thread::Builder::new()
            .name("xrpld-peerfinder".to_owned())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        tracing::error!(target: "peerfinder", %error,
                            "PeerFinder runtime failed to start");
                        return;
                    }
                };
                runtime.block_on(run_live_peerfinder(
                    overlay,
                    fixed_endpoints,
                    endpoints,
                    peerfinder_bootcache_path,
                    auto_connect,
                    target_outbound_peers,
                    network_ops_mode_owner,
                    status_rpc_state,
                ));
            })
            .map_err(|error| {
                self.rollback_started(format!("PeerFinder worker start failed: {error}"))
            })?;
        *self
            .peerfinder_thread
            .lock()
            .expect("overlay PeerFinder worker mutex must not be poisoned") =
            Some(peerfinder_thread);

        if let Some(listener_setup) = self.listener_setup.as_ref()
            && !self.server_owns_listener
        {
            let address = format!("{}:{}", listener_setup.ip, listener_setup.port)
                .parse::<SocketAddr>()
                .map_err(|error| {
                    self.rollback_started(format!("invalid overlay listener address: {error}"))
                })?;
            let listener = StdTcpListener::bind(address).map_err(|error| {
                self.rollback_started(format!("overlay peer listener bind failed: {error}"))
            })?;
            listener.set_nonblocking(true).map_err(|error| {
                self.rollback_started(format!(
                    "overlay peer listener nonblocking setup failed: {error}"
                ))
            })?;
            let listener = tokio::net::TcpListener::from_std(listener).map_err(|error| {
                self.rollback_started(format!("overlay peer listener adoption failed: {error}"))
            })?;
            let acceptor = self.overlay.bind(listener).map_err(|error| {
                self.rollback_started(format!("overlay peer listener TLS setup failed: {error}"))
            })?;
            let task = self.overlay.spawn_listener(acceptor);
            *self
                .listener_task
                .lock()
                .expect("overlay listener task mutex must not be poisoned") = Some(task);
        }

        Ok(())
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.overlay.signal_stop();
        if let Some(task) = self
            .listener_task
            .lock()
            .expect("overlay listener task mutex must not be poisoned")
            .take()
        {
            task.abort();
        }
        if let Some(worker) = self
            .peerfinder_thread
            .lock()
            .expect("overlay PeerFinder worker mutex must not be poisoned")
            .take()
        {
            let _ = worker.join();
        }
        self.overlay.wait_for_session_shutdown();
    }

    fn fd_required(&self) -> usize {
        if self.server_owns_listener {
            0
        } else {
            self.listener_setup
                .as_ref()
                .map_or(0, ServerPortOverlaySetup::fd_required)
        }
    }
}

impl AppOverlayRuntime {
    fn rollback_started(&self, error: String) -> String {
        self.overlay.signal_stop();
        if let Some(task) = self
            .listener_task
            .lock()
            .expect("overlay listener task mutex must not be poisoned")
            .take()
        {
            task.abort();
        }
        if let Some(worker) = self
            .peerfinder_thread
            .lock()
            .expect("overlay PeerFinder worker mutex must not be poisoned")
            .take()
        {
            let _ = worker.join();
        }
        self.started.store(false, Ordering::Release);
        error
    }
}

enum PeerfinderConnectionEvent {
    Success {
        address: SocketAddr,
        fixed: bool,
    },
    Failure {
        address: SocketAddr,
        fixed: bool,
    },
    Redirect {
        address: SocketAddr,
        fixed: bool,
        peers: Vec<SocketAddr>,
    },
    Closed {
        address: SocketAddr,
        fixed: bool,
    },
}

/// Per-endpoint DNS resolution state owned by the live peerfinder.
///
/// Resolution is always performed by a background task; the autoconnect tick
/// never awaits `lookup_host`. Last-known-good addresses remain dialable while
/// a re-resolution is in flight or after a failure cooldown, so slow or failing
/// DNS cannot stall periodic dialing of already-resolved endpoints.
struct PeerfinderDnsState {
    addresses: Option<Vec<SocketAddr>>,
    last_resolved: Option<Instant>,
    next_attempt: Instant,
}

struct PeerfinderDnsCache {
    resolved: HashMap<String, PeerfinderDnsState>,
    in_flight: HashSet<String>,
}

impl PeerfinderDnsCache {
    fn new() -> Self {
        Self {
            resolved: HashMap::new(),
            in_flight: HashSet::new(),
        }
    }

    /// Returns `true` when a background resolution should be spawned for
    /// `endpoint`: it is not already in flight and its cached outcome is stale
    /// (never resolved, past TTL, or past the failure cooldown).
    fn refresh(&mut self, endpoint: &str, now: Instant) -> bool {
        if self.in_flight.contains(endpoint) {
            return false;
        }
        let due = match self.resolved.get(endpoint) {
            None => true,
            Some(state) => {
                let stale = state
                    .last_resolved
                    .is_none_or(|resolved| now.duration_since(resolved) >= PEERFINDER_DNS_TTL);
                stale && now >= state.next_attempt
            }
        };
        if due {
            self.in_flight.insert(endpoint.to_owned());
        }
        due
    }

    /// Records the outcome of a background resolution. Success refreshes the
    /// served addresses and the TTL; failure keeps any last-known-good
    /// addresses and arms the failure cooldown before the next attempt.
    fn record(&mut self, endpoint: String, outcome: Result<Vec<SocketAddr>, String>, now: Instant) {
        self.in_flight.remove(&endpoint);
        let state = self
            .resolved
            .entry(endpoint)
            .or_insert_with(|| PeerfinderDnsState {
                addresses: None,
                last_resolved: None,
                next_attempt: now,
            });
        match outcome {
            Ok(addresses) => {
                state.addresses = Some(addresses);
                state.last_resolved = Some(now);
                state.next_attempt = now + PEERFINDER_DNS_TTL;
            }
            Err(error) => {
                tracing::info!(target: "peerfinder", %error, "PeerFinder DNS resolution failed");
                state.next_attempt = now + PEERFINDER_DNS_FAILURE_COOLDOWN;
            }
        }
    }

    /// Last-known-good addresses for `endpoint`, if any. A `None` result only
    /// means the endpoint has never resolved successfully; it does not block
    /// the tick or other endpoints.
    fn addresses(&self, endpoint: &str) -> Option<&[SocketAddr]> {
        self.resolved
            .get(endpoint)
            .and_then(|state| state.addresses.as_deref())
    }
}

async fn peerfinder_resolve_endpoint(endpoint: &str) -> Result<Vec<SocketAddr>, String> {
    match tokio::time::timeout(
        PEERFINDER_DNS_RESOLVE_TIMEOUT,
        tokio::net::lookup_host(endpoint),
    )
    .await
    {
        Ok(Ok(addresses)) => Ok(addresses.collect()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("DNS resolution timed out".to_owned()),
    }
}

fn spawn_peerfinder_dns_resolve(
    endpoint: String,
    results: tokio::sync::mpsc::UnboundedSender<(String, Result<Vec<SocketAddr>, String>)>,
    semaphore: Arc<tokio::sync::Semaphore>,
) {
    tokio::spawn(async move {
        let _permit = semaphore
            .acquire_owned()
            .await
            .expect("peerfinder DNS semaphore closed");
        let outcome = peerfinder_resolve_endpoint(&endpoint).await;
        let _ = results.send((endpoint, outcome));
    });
}

async fn load_peerfinder_caches(path: Option<PathBuf>) -> PeerfinderCaches {
    let now = Instant::now();
    let Some(path) = path else {
        return PeerfinderCaches::loaded(Vec::new(), now);
    };
    let display_path = path.display().to_string();
    let loaded = tokio::task::spawn_blocking(move || {
        rdb::PeerFinderDb::open(&path).and_then(|db| db.load_bootcache())
    })
    .await;
    match loaded {
        Ok(Ok(entries)) => {
            tracing::info!(target: "peerfinder", count = entries.len(), path = %display_path,
                "Bootcache loaded into live overlay runtime");
            PeerfinderCaches::loaded(entries, now)
        }
        Ok(Err(error)) => {
            tracing::warn!(target: "peerfinder", path = %display_path, %error,
                "Bootcache load failed; continuing with an empty cache");
            PeerfinderCaches::loaded(Vec::new(), now)
        }
        Err(error) => {
            tracing::warn!(target: "peerfinder", path = %display_path, %error,
                "Bootcache load worker failed; continuing with an empty cache");
            PeerfinderCaches::loaded(Vec::new(), now)
        }
    }
}

async fn persist_peerfinder_bootcache(
    path: Option<PathBuf>,
    entries: Vec<rdb::PeerFinderBootcacheEntry>,
) -> bool {
    let Some(path) = path else {
        return true;
    };
    let display_path = path.display().to_string();
    match tokio::task::spawn_blocking(move || {
        rdb::PeerFinderDb::open(&path).and_then(|db| db.save_bootcache(&entries))
    })
    .await
    {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            tracing::warn!(target: "peerfinder", path = %display_path, %error,
                "Bootcache save failed");
            false
        }
        Err(error) => {
            tracing::warn!(target: "peerfinder", path = %display_path, %error,
                "Bootcache save worker failed");
            false
        }
    }
}

fn spawn_peerfinder_connect(
    overlay: Arc<OverlayImpl>,
    address: SocketAddr,
    fixed: bool,
    results: tokio::sync::mpsc::UnboundedSender<PeerfinderConnectionEvent>,
) {
    // Register the attempt synchronously before spawning, so successive fixed
    // endpoints that resolve to the same IP cannot both start a dial.
    let connect = overlay.connect(address);
    tokio::spawn(async move {
        match connect.await {
            Ok(mut result) => {
                if let Some(session) = result.session.take() {
                    overlay.spawn_peer_session(Arc::clone(&result.peer), session);
                }
                tracing::info!(target: "overlay", %address, peer_id = result.peer.id(), fixed,
                    "PeerFinder connected");
                let _ = results.send(PeerfinderConnectionEvent::Success { address, fixed });
            }
            Err(overlay::ConnectAttemptError::DuplicateOutboundAttempt) => {
                tracing::debug!(target: "overlay", %address, fixed,
                    "PeerFinder duplicate endpoint dial suppressed");
            }
            Err(overlay::ConnectAttemptError::Redirect(peers)) => {
                tracing::info!(target: "overlay", %address, redirect_count = peers.len(), fixed,
                    "PeerFinder connection redirected");
                let _ = results.send(PeerfinderConnectionEvent::Redirect {
                    address,
                    fixed,
                    peers,
                });
            }
            Err(error) => {
                tracing::info!(target: "overlay", %address, fixed, %error,
                    "PeerFinder connection failed");
                let _ = results.send(PeerfinderConnectionEvent::Failure { address, fixed });
            }
        }
    });
}

async fn run_live_peerfinder(
    overlay: Arc<OverlayImpl>,
    fixed_endpoints: Vec<String>,
    bootstrap_endpoints: Vec<String>,
    bootcache_path: Option<PathBuf>,
    auto_connect: bool,
    target_outbound_peers: usize,
    network_ops_mode_owner: Option<AppNetworkOpsModeOwner>,
    status_rpc_state: Option<Arc<StatusRpcState>>,
) {
    let mut caches = load_peerfinder_caches(bootcache_path.clone()).await;
    let (dns_tx, mut dns_rx) = tokio::sync::mpsc::unbounded_channel();
    let dns_semaphore = Arc::new(tokio::sync::Semaphore::new(PEERFINDER_DNS_MAX_CONCURRENT));
    let mut dns_cache = PeerfinderDnsCache::new();
    let mut recent_attempts = HashMap::<IpAddr, Instant>::new();
    let mut fixed_retry_state = HashMap::<SocketAddr, (usize, Instant)>::new();
    let (result_tx, mut result_rx) = tokio::sync::mpsc::unbounded_channel();
    let outbound_failure_tx = result_tx.clone();
    overlay.set_outbound_peer_failure_handler(move |address, fixed| {
        let _ = outbound_failure_tx.send(PeerfinderConnectionEvent::Failure { address, fixed });
    });
    let outbound_close_tx = result_tx.clone();
    overlay.set_outbound_peer_close_handler(move |address, fixed| {
        let _ = outbound_close_tx.send(PeerfinderConnectionEvent::Closed { address, fixed });
    });
    let mut ticker = tokio::time::interval(PEERFINDER_AUTOCONNECT_INTERVAL);
    let mut stop_requested = overlay.stop_receiver();
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // rippled arms its first periodic callback 1 second after setup
    // (OverlayImpl.cpp:131-161). tokio::time::interval fires immediately;
    // consume the first instant tick to match rippled's initial delay.
    ticker.tick().await;

    while !overlay.is_stopping() {
        tokio::select! {
            biased;
            changed = stop_requested.changed() => {
                let _ = changed;
                break;
            }
            _ = ticker.tick() => {}
        }
        if overlay.is_stopping() {
            break;
        }
        let now = Instant::now();
        refresh_peer_count_and_operating_mode(
            overlay.as_ref(),
            status_rpc_state.as_ref(),
            network_ops_mode_owner.as_ref(),
        );

        // Apply DNS resolution results completed since the last tick before
        // deciding what to dial; the tick itself never awaits `lookup_host`.
        while let Ok((endpoint, outcome)) = dns_rx.try_recv() {
            dns_cache.record(endpoint, outcome, now);
        }

        // `Logic::oncePerSecond`: expire live data, retry squelches, and
        // check the bootcache persistence cooldown on every timer tick.
        caches.expire_livecache(now);
        prune_recent_bootcache_attempts(&mut recent_attempts, now);
        for endpoints in overlay.take_endpoints() {
            for endpoint in endpoints.endpoints {
                caches.on_learned_endpoint(endpoint.endpoint, endpoint.hops, now);
            }
        }
        while let Ok(event) = result_rx.try_recv() {
            match event {
                PeerfinderConnectionEvent::Success { address, fixed } => {
                    caches.on_success(address);
                    if fixed {
                        fixed_retry_state.remove(&address);
                    }
                }
                PeerfinderConnectionEvent::Failure { address, fixed } => {
                    caches.on_failure(address);
                    if fixed {
                        let failures = fixed_retry_state
                            .get(&address)
                            .map(|(failures, _)| failures.saturating_add(1))
                            .unwrap_or(1);
                        fixed_retry_state
                            .insert(address, (failures, now + fixed_retry_delay(failures)));
                    }
                }
                PeerfinderConnectionEvent::Closed { address, fixed } => {
                    // Mirrors PeerFinder::onClosed(slot): release the live
                    // connection/attempt state without recording a valence
                    // failure. A fixed peer is eligible for its normal retry
                    // policy immediately after its slot disappears.
                    recent_attempts.remove(&address.ip());
                    if fixed {
                        fixed_retry_state.remove(&address);
                    }
                }
                PeerfinderConnectionEvent::Redirect {
                    address,
                    fixed,
                    peers,
                } => {
                    // A redirect supplies referrals through `onRedirects`,
                    // while this outbound attempt itself did not complete the
                    // handshake and therefore reaches `onFailure` when its
                    // connect slot closes in rippled.
                    caches.on_failure(address);
                    caches.on_redirects(peers);
                    if fixed {
                        let failures = fixed_retry_state
                            .get(&address)
                            .map(|(failures, _)| failures.saturating_add(1))
                            .unwrap_or(1);
                        fixed_retry_state
                            .insert(address, (failures, now + fixed_retry_delay(failures)));
                    }
                }
            }
        }

        // OverlayImpl::Timer invokes PeerFinder::autoconnect every second.
        // `Logic::autoconnect` gives fixed peers the first chance to spend
        // the one global 20-attempt budget, and does not enter automatic
        // livecache/bootcache selection while fixed work remains in flight.
        let mut fixed_attempts_started = 0;
        let mut fixed_addresses = Vec::new();
        for endpoint in &fixed_endpoints {
            if dns_cache.refresh(endpoint, now) {
                spawn_peerfinder_dns_resolve(
                    endpoint.clone(),
                    dns_tx.clone(),
                    Arc::clone(&dns_semaphore),
                );
            }
            let Some(addresses) = dns_cache.addresses(endpoint) else {
                continue;
            };
            overlay.remember_fixed_peer_endpoints(addresses.iter().copied());
            fixed_addresses.extend(addresses.iter().copied());
        }
        let active_fixed_peers = overlay.active_fixed_peers_count();
        let fixed_peer_slots = overlay.fixed_peer_slot_count();
        if active_fixed_peers < fixed_peer_slots {
            let mut attempt_budget = peerfinder_attempt_budget(overlay.pending_outbound_attempts());
            for address in fixed_addresses {
                if attempt_budget == 0 {
                    break;
                }
                if fixed_retry_state_or_due(&fixed_retry_state, address, now).1 > now
                    || overlay.outbound_endpoint_is_active_or_pending(address)
                {
                    continue;
                }
                // `connect` reserves the IP synchronously, so the next
                // iteration observes same-IP fixed aliases as pending.
                spawn_peerfinder_connect(Arc::clone(&overlay), address, true, result_tx.clone());
                fixed_attempts_started += 1;
                attempt_budget -= 1;
            }
        }

        let fixed_stage_blocks_automatic = fixed_stage_blocks_automatic_dials(
            active_fixed_peers,
            fixed_peer_slots,
            fixed_attempts_started,
            overlay.pending_outbound_attempts(),
        );
        if !fixed_stage_blocks_automatic {
            if auto_connect {
                for endpoint in &bootstrap_endpoints {
                    if dns_cache.refresh(endpoint, now) {
                        spawn_peerfinder_dns_resolve(
                            endpoint.clone(),
                            dns_tx.clone(),
                            Arc::clone(&dns_semaphore),
                        );
                    }
                    if let Some(addresses) = dns_cache.addresses(endpoint) {
                        for address in addresses {
                            caches.insert_static_bootcache(*address);
                        }
                    }
                }
            }

            let active_outbound = overlay.active_outbound_peers_count();
            if bootstrap_can_dial_bootcache(auto_connect, active_outbound, target_outbound_peers) {
                let connected_ips = overlay
                    .active_peers()
                    .into_iter()
                    .map(|peer| peer.remote_address().ip())
                    .collect::<BTreeSet<_>>();
                let attempt_budget = peerfinder_attempt_budget(overlay.pending_outbound_attempts());
                let livecache_candidates =
                    caches.select_livecache(&connected_ips, &recent_attempts, now, attempt_budget);
                let livecache_attempts_started = livecache_candidates.len();
                for address in livecache_candidates {
                    recent_attempts.insert(address.ip(), now + PEERFINDER_RECENT_ATTEMPT_DURATION);
                    spawn_peerfinder_connect(
                        Arc::clone(&overlay),
                        address,
                        false,
                        result_tx.clone(),
                    );
                }

                if !livecache_stage_blocks_bootcache(
                    livecache_attempts_started,
                    overlay.pending_outbound_attempts(),
                ) {
                    let bootcache_candidates = select_bootcache_endpoints(
                        &connected_ips,
                        &caches.bootcache,
                        &recent_attempts,
                        now,
                        peerfinder_attempt_budget(overlay.pending_outbound_attempts()),
                    );
                    for address in bootcache_candidates {
                        recent_attempts
                            .insert(address.ip(), now + PEERFINDER_RECENT_ATTEMPT_DURATION);
                        spawn_peerfinder_connect(
                            Arc::clone(&overlay),
                            address,
                            false,
                            result_tx.clone(),
                        );
                    }
                }
            }
        }

        if let Some(entries) = caches.take_update_if_due(now, false)
            && !persist_peerfinder_bootcache(bootcache_path.clone(), entries).await
        {
            // Preserve the update request so a transient SQLite fault is not
            // converted into a silently volatile cache.
            caches.flag_for_update();
        }
    }

    // `Bootcache::~Bootcache` calls `update()` without consulting the
    // cooldown. Mirror that final durable flush on orderly overlay shutdown.
    if let Some(entries) = caches.take_update_if_due(Instant::now(), true) {
        let _ = persist_peerfinder_bootcache(bootcache_path, entries).await;
    }
}

fn refresh_peer_count_and_operating_mode(
    overlay: &OverlayImpl,
    status_rpc_state: Option<&Arc<StatusRpcState>>,
    network_ops_mode_owner: Option<&AppNetworkOpsModeOwner>,
) {
    let peer_count = u32::try_from(overlay.active_peers().len()).unwrap_or(u32::MAX);

    if let Some(state) = status_rpc_state {
        state.set_peer_count(Some(peer_count));
    }

    // Overlay is the peer-availability fact source. A 0→positive peer count
    // arms the availability timestamp the acquisition actor reads back at its
    // first outbound request (peer-availability-to-first-request latency).
    if peer_count > 0 {
        xrpld_metrics::acquisition::note_peers_available();
    } else {
        xrpld_metrics::acquisition::note_peers_unavailable();
    }

    if peer_count > 0
        && let Some(state) = network_ops_mode_owner
        && matches!(
            state.operating_mode(),
            NetworkOpsOperatingMode::Disconnected
        )
    {
        let _ = state
            .set_operating_mode_with_reason(NetworkOpsOperatingMode::Connected, "peers_available");
    }
}

pub fn build_overlay_setup(config: &BasicConfig) -> Result<Setup, String> {
    install_tls_provider();

    let mut setup = Setup {
        client_config: Some(default_overlay_client_config()?),
        server_config: None,
        server_ssl_acceptor: None,
        public_ip: None,
        fixed_peer_ips: std::collections::HashSet::new(),
        ip_limit: 0,
        peer_limit: 0,
        peer_limit_in: None,
        peer_limit_out: None,
        want_incoming: true,
        verify_endpoints: true,
        crawl_options: CRAWL_OPTION_OVERLAY | CRAWL_OPTION_SERVER_INFO | CRAWL_OPTION_UNL,
        network_id: None,
        vl_enabled: true,
        tx_reduce_relay_enabled: false,
        tx_reduce_relay_min_peers: 20,
        tx_relay_percentage: 25,
        vp_reduce_relay_base_squelch_enabled: false,
        vp_reduce_relay_max_selected_peers: 5,
        reduce_relay_wait: DEFAULT_REDUCE_RELAY_WAIT,
    };

    parse_peer_limit_sections(config, &mut setup)?;
    parse_overlay_section(config.section("overlay"), &mut setup)?;
    parse_crawl_section(config.section("crawl"), &mut setup)?;
    parse_vl_section(config.section("vl"), &mut setup)?;
    parse_reduce_relay_section(config.section("reduce_relay"), &mut setup)?;
    setup.network_id = parse_network_id(config)?;

    Ok(setup)
}

pub fn build_overlay_runtime(
    config: &BasicConfig,
    server_ports_setup: Option<&ServerPortsSetup>,
    handoff: Arc<dyn OverlayHandoff>,
    network_ops_mode_owner: Option<AppNetworkOpsModeOwner>,
    status_rpc_state: Option<Arc<StatusRpcState>>,
) -> Result<Arc<AppOverlayRuntime>, String> {
    let mut setup = build_overlay_setup(config)?;
    let peer_private = parse_peer_private(config)?;
    let fixed_peer_endpoints = parse_peer_endpoints(config, "ips_fixed")?;
    let bootstrap_peer_endpoints = parse_bootstrap_peer_endpoints(config, &fixed_peer_endpoints)?;
    let peerfinder_bootcache_path = config
        .legacy("database_path")
        .ok()
        .map(|path| PathBuf::from(path).join("peerfinder.db"));
    let listener_setup = server_ports_setup.and_then(|setup| setup.overlay.clone());
    let server_owns_listener = server_ports_setup.is_some_and(|setup| {
        setup.ports.iter().any(|port| {
            port.allows_peer()
                && (port.allows_http() || port.allows_websocket())
                && listener_setup
                    .as_ref()
                    .is_some_and(|listener| port.ip == listener.ip && port.port == listener.port)
        })
    });
    if let Some(listener) = listener_setup.as_ref() {
        setup.server_config = build_overlay_server_config(listener)?;
        setup.server_ssl_acceptor = build_overlay_ssl_acceptor(listener)?;
    }
    setup.want_incoming = listener_setup.is_some() && !peer_private;
    setup.fixed_peer_ips = parse_fixed_peer_ips(&fixed_peer_endpoints);
    let overlay = Arc::new(OverlayImpl::new(setup, handoff).map_err(|error| error.to_string())?);
    Ok(Arc::new(AppOverlayRuntime::new(
        overlay,
        listener_setup,
        server_owns_listener,
        fixed_peer_endpoints,
        bootstrap_peer_endpoints,
        !peer_private,
        peerfinder_bootcache_path,
        network_ops_mode_owner,
        status_rpc_state,
    )))
}

fn parse_peer_endpoints(config: &BasicConfig, section_name: &str) -> Result<Vec<String>, String> {
    let mut endpoints = Vec::new();
    for raw in config.section(section_name).values() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }

        let parts = line.split_whitespace().collect::<Vec<_>>();
        let endpoint = match parts.as_slice() {
            [host] => {
                if host.contains(':') {
                    (*host).to_owned()
                } else {
                    format!("{}:{}", host, DEFAULT_PEER_PORT)
                }
            }
            [host, port] => {
                let parsed = port
                    .parse::<u16>()
                    .map_err(|_| format!("invalid peer port in [{section_name}]: {line}"))?;
                format!("{}:{}", host, parsed)
            }
            _ => return Err(format!("invalid [{section_name}] entry: {line}")),
        };

        if !endpoints.contains(&endpoint) {
            endpoints.push(endpoint);
        }
    }
    Ok(endpoints)
}

fn parse_bootstrap_peer_endpoints(
    config: &BasicConfig,
    fixed_peer_endpoints: &[String],
) -> Result<Vec<String>, String> {
    let configured = parse_peer_endpoints(config, "ips")?;
    if !configured.is_empty() {
        return Ok(configured);
    }
    if !fixed_peer_endpoints.is_empty() {
        return Ok(fixed_peer_endpoints.to_vec());
    }
    Ok(DEFAULT_BOOTSTRAP_PEER_ENDPOINTS
        .iter()
        .map(|endpoint| (*endpoint).to_owned())
        .collect())
}

fn parse_fixed_peer_ips(fixed_peer_endpoints: &[String]) -> std::collections::HashSet<IpAddr> {
    fn canonical_ip(ip: IpAddr) -> IpAddr {
        match ip {
            IpAddr::V6(ipv6) => ipv6
                .to_ipv4_mapped()
                .map(IpAddr::V4)
                .unwrap_or(IpAddr::V6(ipv6)),
            IpAddr::V4(_) => ip,
        }
    }

    fixed_peer_endpoints
        .iter()
        .filter_map(|endpoint| endpoint.parse::<SocketAddr>().ok())
        .map(|endpoint| canonical_ip(endpoint.ip()))
        .collect()
}

fn install_tls_provider() {
    static TLS_PROVIDER: OnceLock<()> = OnceLock::new();

    TLS_PROVIDER.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn default_overlay_client_config() -> Result<Arc<rustls::ClientConfig>, String> {
    let identity = anonymous_tls_identity_der().map_err(|error| error.to_string())?;
    let cert_chain = rustls_cert_chain(&identity);
    let private_key = rustls_private_key(&identity);

    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoServerVerification))
        .with_client_auth_cert(cert_chain, private_key)
        .map(Arc::new)
        .map_err(|error| error.to_string())
}

fn build_overlay_server_config(
    listener: &ServerPortOverlaySetup,
) -> Result<Option<Arc<rustls::ServerConfig>>, String> {
    // The XRP Ledger peer protocol always uses TLS. When no explicit certs are
    // provided (or the port is not marked "secure"), generate an anonymous
    // self-signed identity for the listener.
    let identity = if listener.ssl_key.is_empty()
        && listener.ssl_cert.is_empty()
        && listener.ssl_chain.is_empty()
    {
        anonymous_tls_identity_der()
    } else {
        authenticated_tls_identity_der(&listener.ssl_key, &listener.ssl_cert, &listener.ssl_chain)
    }
    .map_err(|error| error.to_string())?;

    let cert_chain = rustls_cert_chain(&identity);
    let private_key = rustls_private_key(&identity);

    rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, private_key)
        .map(|config| Some(Arc::new(config)))
        .map_err(|error| error.to_string())
}

fn rustls_cert_chain(identity: &TlsIdentityDer) -> Vec<CertificateDer<'static>> {
    identity
        .certificate_chain_der()
        .into_iter()
        .map(CertificateDer::from)
        .collect()
}

fn rustls_private_key(identity: &TlsIdentityDer) -> PrivateKeyDer<'static> {
    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        identity.private_key_pkcs8_der().to_vec(),
    ))
}

/// Build an OpenSSL `SslAcceptor` for inbound peer connections.
///
/// This is required because the XRPL peer protocol derives a Session-Signature
/// from raw TLS Finished messages (`SSL_get_finished` / `SSL_get_peer_finished`),
/// which rustls does not expose. The openssl crate provides direct access.
fn build_overlay_ssl_acceptor(
    listener: &ServerPortOverlaySetup,
) -> Result<Option<Arc<openssl::ssl::SslAcceptor>>, String> {
    use openssl::pkey::PKey;
    use openssl::ssl::{SslAcceptor, SslMethod, SslVerifyMode};
    use openssl::x509::X509;

    let identity = if listener.ssl_key.is_empty()
        && listener.ssl_cert.is_empty()
        && listener.ssl_chain.is_empty()
    {
        anonymous_tls_identity_der()
    } else {
        authenticated_tls_identity_der(&listener.ssl_key, &listener.ssl_cert, &listener.ssl_chain)
    }
    .map_err(|error| error.to_string())?;

    let mut builder =
        SslAcceptor::mozilla_intermediate(SslMethod::tls()).map_err(|e| e.to_string())?;

    // Set certificate
    let cert_der = identity
        .certificate_chain_der()
        .first()
        .ok_or_else(|| "no certificate in identity".to_owned())?
        .clone();
    let cert = X509::from_der(&cert_der).map_err(|e| e.to_string())?;
    builder.set_certificate(&cert).map_err(|e| e.to_string())?;

    // Set additional chain certs
    for chain_cert_der in identity.certificate_chain_der().iter().skip(1) {
        let chain_cert = X509::from_der(chain_cert_der).map_err(|e| e.to_string())?;
        builder
            .add_extra_chain_cert(chain_cert)
            .map_err(|e| e.to_string())?;
    }

    // Set private key
    let key = PKey::private_key_from_pkcs8(identity.private_key_pkcs8_der())
        .map_err(|e| e.to_string())?;
    builder.set_private_key(&key).map_err(|e| e.to_string())?;

    // No client auth (matching rippled)
    builder.set_verify(SslVerifyMode::NONE);

    Ok(Some(Arc::new(builder.build())))
}

#[cfg(test)]
pub(crate) fn overlay_server_config(
    listener: &ServerPortOverlaySetup,
) -> Result<Option<Arc<rustls::ServerConfig>>, String> {
    install_tls_provider();
    build_overlay_server_config(listener)
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn test_default_overlay_client_config() -> Result<Arc<rustls::ClientConfig>, String> {
    install_tls_provider();
    default_overlay_client_config()
}

fn parse_peer_limit_sections(config: &BasicConfig, setup: &mut Setup) -> Result<(), String> {
    let max_peers = parse_single_section_usize(config, "peers_max")?;
    let inbound_max = parse_single_section_usize(config, "peers_in_max")?;
    let outbound_max = parse_single_section_usize(config, "peers_out_max")?;

    if let Some(max_peers) = max_peers {
        // rippled gives legacy [peers_max] precedence over paired directional
        // sections, including when its value is zero (the default then applies).
        setup.peer_limit = max_peers;
        setup.peer_limit_in = None;
        setup.peer_limit_out = None;
        return Ok(());
    }

    match (inbound_max, outbound_max) {
        (None, None) => Ok(()),
        (Some(_), None) | (None, Some(_)) => {
            Err("Both [peers_in_max] and [peers_out_max] must be configured".to_owned())
        }
        (Some(inbound_max), Some(outbound_max)) => {
            if inbound_max > 1_000 {
                return Err("Inbound peer limit must be less than or equal to 1000".to_owned());
            }
            if !(10..=1_000).contains(&outbound_max) {
                return Err("Outbound peer limit must be in the range 10-1000".to_owned());
            }
            setup.peer_limit_in = Some(inbound_max);
            setup.peer_limit_out = Some(outbound_max);
            Ok(())
        }
    }
}

fn parse_single_section_usize(config: &BasicConfig, name: &str) -> Result<Option<usize>, String> {
    let values = config.section(name).values();
    match values {
        [] => Ok(None),
        [value] => value
            .trim()
            .parse::<usize>()
            .map(Some)
            .map_err(|_| format!("Configured [{name}] section is invalid")),
        _ => Err(format!("Configured [{name}] section has too many values")),
    }
}

fn parse_peer_private(config: &BasicConfig) -> Result<bool, String> {
    let values = config.section("peer_private").values();
    match values {
        [] => Ok(false),
        [value] => parse_bool(value.clone())
            .map_err(|_| "Configured [peer_private] section is invalid".to_owned()),
        _ => Err("Configured [peer_private] section has too many values".to_owned()),
    }
}

fn parse_overlay_section(section: &Section, setup: &mut Setup) -> Result<(), String> {
    if let Some(limit) = raw(section, "ip_limit") {
        let parsed = limit
            .parse::<usize>()
            .map_err(|_| "Configured IP limit is invalid".to_owned())?;
        setup.ip_limit = parsed;
    }

    if let Some(verify_endpoints) = raw(section, "verify_endpoints") {
        setup.verify_endpoints = parse_bool(verify_endpoints)
            .map_err(|_| "Configured verify_endpoints is invalid".to_owned())?;
    }

    if let Some(public_ip) = raw(section, "public_ip") {
        if public_ip.is_empty() {
            return Ok(());
        }

        let parsed = public_ip
            .parse::<IpAddr>()
            .map_err(|_| "Configured public IP is invalid".to_owned())?;
        if !is_public_ip(parsed) {
            return Err("Configured public IP is invalid".to_owned());
        }
        setup.public_ip = Some(parsed);
    }
    Ok(())
}

fn parse_crawl_section(section: &Section, setup: &mut Setup) -> Result<(), String> {
    let values = section.values();
    if values.len() > 1 {
        return Err("Configured [crawl] section is invalid, too many values".to_owned());
    }

    let crawl_enabled = match values {
        [] => true,
        [value] => parse_crawl_enable(value)?,
        _ => unreachable!("crawl size checked"),
    };
    if !crawl_enabled {
        setup.crawl_options = CRAWL_OPTION_DISABLED;
        return Ok(());
    }

    setup.crawl_options = CRAWL_OPTION_DISABLED;
    if section_bool(section, "overlay")?.unwrap_or(true) {
        setup.crawl_options |= CRAWL_OPTION_OVERLAY;
    }
    if section_bool(section, "server")?.unwrap_or(true) {
        setup.crawl_options |= CRAWL_OPTION_SERVER_INFO;
    }
    if section_bool(section, "counts")?.unwrap_or(false) {
        setup.crawl_options |= CRAWL_OPTION_SERVER_COUNTS;
    }
    if section_bool(section, "unl")?.unwrap_or(true) {
        setup.crawl_options |= CRAWL_OPTION_UNL;
    }
    Ok(())
}

fn parse_vl_section(section: &Section, setup: &mut Setup) -> Result<(), String> {
    if let Some(enabled) = section_bool(section, "enabled")? {
        setup.vl_enabled = enabled;
    }
    Ok(())
}

fn parse_reduce_relay_section(section: &Section, setup: &mut Setup) -> Result<(), String> {
    if section.exists("vp_base_squelch_enable") {
        setup.vp_reduce_relay_base_squelch_enabled =
            section_bool_required(section, "vp_base_squelch_enable")?;
    } else if section.exists("vp_enable") {
        setup.vp_reduce_relay_base_squelch_enabled = section_bool_required(section, "vp_enable")?;
    } else {
        setup.vp_reduce_relay_base_squelch_enabled = false;
    }

    setup.vp_reduce_relay_max_selected_peers = raw(section, "vp_base_squelch_max_selected_peers")
        .map(|value| {
            value.parse::<u16>().map_err(|_| {
                "Invalid reduce_relay vp_base_squelch_max_selected_peers must be greater than or equal to 3"
                    .to_owned()
            })
        })
        .transpose()?
        .unwrap_or(5);
    if setup.vp_reduce_relay_max_selected_peers < 3 {
        return Err(
            "Invalid reduce_relay vp_base_squelch_max_selected_peers must be greater than or equal to 3"
                .to_owned(),
        );
    }

    setup.tx_reduce_relay_enabled = section_bool(section, "tx_enable")?.unwrap_or(false);
    setup.tx_reduce_relay_min_peers = raw(section, "tx_min_peers")
        .map(|value| {
            value.parse::<usize>().map_err(|_| {
                "Invalid reduce_relay, tx_min_peers must be greater than or equal to 10, tx_relay_percentage must be greater than or equal to 10 and less than or equal to 100"
                    .to_owned()
            })
        })
        .transpose()?
        .unwrap_or(20);
    setup.tx_relay_percentage = raw(section, "tx_relay_percentage")
        .map(|value| {
            value.parse::<usize>().map_err(|_| {
                "Invalid reduce_relay, tx_min_peers must be greater than or equal to 10, tx_relay_percentage must be greater than or equal to 10 and less than or equal to 100"
                    .to_owned()
            })
        })
        .transpose()?
        .unwrap_or(25);
    if !(10..=100).contains(&setup.tx_relay_percentage) || setup.tx_reduce_relay_min_peers < 10 {
        return Err(
            "Invalid reduce_relay, tx_min_peers must be greater than or equal to 10, tx_relay_percentage must be greater than or equal to 10 and less than or equal to 100"
                .to_owned(),
        );
    }

    Ok(())
}

fn parse_network_id(config: &BasicConfig) -> Result<Option<u32>, String> {
    let id = config.legacy("network_id").unwrap_or_default();
    if id.is_empty() {
        return Ok(None);
    }

    let canonical = match id.as_str() {
        "main" => "0",
        "testnet" => "1",
        "devnet" => "2",
        value => value,
    };

    canonical.parse::<u32>().map(Some).map_err(|_| {
        "Configured [network_id] section is invalid: must be a number or one of the strings 'main', 'testnet' or 'devnet'."
            .to_owned()
    })
}

fn section_bool(section: &Section, name: &str) -> Result<Option<bool>, String> {
    raw(section, name).map(parse_bool).transpose()
}

fn section_bool_required(section: &Section, name: &str) -> Result<bool, String> {
    section_bool(section, name)?.ok_or_else(|| format!("missing boolean field: {name}"))
}

fn parse_bool(value: String) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        other => Err(format!("invalid boolean value: {other}")),
    }
}

fn parse_crawl_enable(value: &str) -> Result<bool, String> {
    match value.trim() {
        "1" => Ok(true),
        "0" => Ok(false),
        other => Err(format!(
            "Configured [crawl] section has invalid value: {other}"
        )),
    }
}

fn raw(section: &Section, name: &str) -> Option<String> {
    section.get::<String>(name).ok().flatten()
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(is_public_ipv4)
            .unwrap_or_else(|| is_public_ipv6(ip)),
    }
}

fn is_public_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let octets = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || (octets[0] == 0)
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 192 && octets[1] == 88 && octets[2] == 99)
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
        || octets[0] >= 240)
}

fn is_public_ipv6(ip: std::net::Ipv6Addr) -> bool {
    let segments = ip.segments();
    let first = segments[0];
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (segments[0] == 0x0100 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0)
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || (segments[0] == 0x2001 && segments[1] == 0x0000)
        || (segments[0] == 0x2001 && (segments[1] & 0xfff0) == 0x0020)
        || segments[0] == 0x2002)
}

#[cfg(test)]
mod tests {
    use super::{
        BOOTCACHE_STATIC_VALENCE, BootcacheEntry, BootstrapOverlayHandoff, CRAWL_OPTION_DISABLED,
        CRAWL_OPTION_OVERLAY, CRAWL_OPTION_SERVER_COUNTS, CRAWL_OPTION_SERVER_INFO,
        CRAWL_OPTION_UNL, PEERFINDER_AUTOCONNECT_INTERVAL, PEERFINDER_DNS_FAILURE_COOLDOWN,
        PEERFINDER_DNS_MAX_CONCURRENT, PEERFINDER_DNS_RESOLVE_TIMEOUT, PEERFINDER_DNS_TTL,
        PeerfinderCaches, PeerfinderDnsCache, bootcache_on_failure, bootcache_on_success,
        bootstrap_can_dial_bootcache, bootstrap_needs_bootcache_dial, build_overlay_runtime,
        build_overlay_setup, default_overlay_client_config, fixed_retry_state_or_due,
        fixed_stage_blocks_automatic_dials, is_public_ip, livecache_stage_blocks_bootcache,
        load_peerfinder_caches, overlay_server_config, parse_bootstrap_peer_endpoints,
        parse_fixed_peer_ips, parse_peer_endpoints, peerfinder_attempt_budget,
        peerfinder_outbound_target, persist_peerfinder_bootcache, remember_bootcache_endpoint,
        select_bootcache_endpoints, spawn_peerfinder_dns_resolve,
    };
    use crate::runtime::main_runtime::ManagedComponent;
    use basics::basic_config::BasicConfig;
    use std::collections::{BTreeSet, HashMap, HashSet};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn config(text: &str) -> BasicConfig {
        let mut config = BasicConfig::new();
        let mut sections = basics::basic_config::IniFileSections::new();
        let mut current = String::new();
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                current = line[1..line.len() - 1].trim().to_owned();
                let _ = sections.entry(current.clone()).or_default();
                continue;
            }
            sections
                .entry(current.clone())
                .or_default()
                .push(raw_line.to_owned());
        }
        config.build(&sections);
        config
    }

    #[test]
    fn overlay_setup_uses_cpp_network_id_aliases_and_defaults() {
        let main = build_overlay_setup(&config("[network_id]\nmain\n")).expect("main");
        let testnet = build_overlay_setup(&config("[network_id]\ntestnet\n")).expect("testnet");
        let devnet = build_overlay_setup(&config("[network_id]\ndevnet\n")).expect("devnet");
        let numeric = build_overlay_setup(&config("[network_id]\n21338\n")).expect("numeric");
        let defaulted = build_overlay_setup(&config("")).expect("default");

        assert_eq!(main.network_id, Some(0));
        assert_eq!(testnet.network_id, Some(1));
        assert_eq!(devnet.network_id, Some(2));
        assert_eq!(numeric.network_id, Some(21_338));
        assert_eq!(defaulted.network_id, None);
        assert!(defaulted.client_config.is_some());
        assert!(!defaulted.tx_reduce_relay_enabled);
        assert_eq!(defaulted.tx_reduce_relay_min_peers, 20);
        assert_eq!(defaulted.tx_relay_percentage, 25);
        assert!(!defaulted.vp_reduce_relay_base_squelch_enabled);
        assert_eq!(defaulted.vp_reduce_relay_max_selected_peers, 5);
        assert_eq!(
            defaulted.crawl_options,
            CRAWL_OPTION_OVERLAY | CRAWL_OPTION_SERVER_INFO | CRAWL_OPTION_UNL
        );
        assert!(defaulted.verify_endpoints);
    }

    #[test]
    fn overlay_setup_rejects_invalid_network_id_and_public_ip() {
        let network_error = build_overlay_setup(&config("[network_id]\nsidechain\n"))
            .err()
            .expect("network id");
        assert_eq!(
            network_error,
            "Configured [network_id] section is invalid: must be a number or one of the strings 'main', 'testnet' or 'devnet'."
        );

        let public_ip_error = build_overlay_setup(&config("[overlay]\npublic_ip = 10.0.0.1\n"))
            .err()
            .expect("public ip");
        assert_eq!(public_ip_error, "Configured public IP is invalid");

        for ip in ["192.88.99.1", "100::1", "2001:20::1", "2001:2f:ffff::1"] {
            let error = build_overlay_setup(&config(&format!("[overlay]\npublic_ip = {ip}\n")))
                .err()
                .expect("special public ip range");
            assert_eq!(error, "Configured public IP is invalid");
        }

        let verify_error = build_overlay_setup(&config("[overlay]\nverify_endpoints = maybe\n"))
            .err()
            .expect("verify endpoints");
        assert_eq!(verify_error, "Configured verify_endpoints is invalid");
    }

    #[test]
    fn overlay_setup_parses_verify_endpoints() {
        let disabled =
            build_overlay_setup(&config("[overlay]\nverify_endpoints = false\n")).expect("false");
        assert!(!disabled.verify_endpoints);

        let enabled =
            build_overlay_setup(&config("[overlay]\nverify_endpoints = 1\n")).expect("true");
        assert!(enabled.verify_endpoints);
    }

    #[test]
    fn overlay_public_ip_classification_matches_cpp_ranges() {
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "2001:4860:4860::8888",
            "::ffff:8.8.4.4",
        ] {
            assert!(
                is_public_ip(ip.parse::<IpAddr>().expect("public ip")),
                "{ip}"
            );
        }

        for ip in [
            "0.1.2.3",
            "10.0.0.1",
            "100.64.0.1",
            "100.127.255.255",
            "127.0.0.1",
            "169.254.1.1",
            "172.16.0.1",
            "192.0.0.1",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.0.1",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::ffff:10.0.0.1",
            "100::1",
            "2001::1",
            "2001:20::1",
            "2001:2f:ffff::1",
            "2001:db8::1",
            "2002::1",
            "fc00::1",
            "fd00::1",
            "fe80::1",
            "ff00::1",
        ] {
            assert!(
                !is_public_ip(ip.parse::<IpAddr>().expect("private ip")),
                "{ip}"
            );
        }
    }

    #[test]
    fn overlay_setup_parses_crawl_and_reduce_relay_sections() {
        let parsed = build_overlay_setup(&config(
            r#"
[crawl]
1
overlay = false
server = true
counts = true
unl = false

[vl]
enabled = false

[reduce_relay]
vp_enable = true
vp_base_squelch_max_selected_peers = 7
tx_enable = true
tx_min_peers = 12
tx_relay_percentage = 40
"#,
        ))
        .expect("parsed");

        assert_eq!(
            parsed.crawl_options,
            CRAWL_OPTION_SERVER_INFO | CRAWL_OPTION_SERVER_COUNTS
        );
        assert!(!parsed.vl_enabled);
        assert!(parsed.vp_reduce_relay_base_squelch_enabled);
        assert_eq!(parsed.vp_reduce_relay_max_selected_peers, 7);
        assert!(parsed.tx_reduce_relay_enabled);
        assert_eq!(parsed.tx_reduce_relay_min_peers, 12);
        assert_eq!(parsed.tx_relay_percentage, 40);
    }

    #[test]
    fn overlay_setup_rejects_invalid_crawl_and_reduce_relay_values() {
        let crawl_error = build_overlay_setup(&config("[crawl]\n2\n"))
            .err()
            .expect("crawl");
        assert_eq!(
            crawl_error,
            "Configured [crawl] section has invalid value: 2"
        );

        let disabled = build_overlay_setup(&config("[crawl]\n0\n")).expect("disabled crawl");
        assert_eq!(disabled.crawl_options, CRAWL_OPTION_DISABLED);

        let relay_error = build_overlay_setup(&config(
            r#"
[reduce_relay]
tx_min_peers = 9
"#,
        ))
        .err()
        .expect("reduce relay");
        assert_eq!(
            relay_error,
            "Invalid reduce_relay, tx_min_peers must be greater than or equal to 10, tx_relay_percentage must be greater than or equal to 10 and less than or equal to 100"
        );
    }

    #[test]
    fn managed_overlay_runtime_reports_listener_budget_and_lifecycle() {
        let runtime = build_overlay_runtime(
            &config(""),
            Some(&crate::ServerPortsSetup {
                ports: Vec::new(),
                client: None,
                overlay: Some(crate::ServerPortOverlaySetup {
                    ip: "127.0.0.1".to_owned(),
                    port: 51235,
                    limit: 64,
                    secure: true,
                    ssl_key: String::new(),
                    ssl_cert: String::new(),
                    ssl_chain: String::new(),
                    ssl_ciphers: String::new(),
                }),
                grpc: None,
            }),
            Arc::new(BootstrapOverlayHandoff),
            None,
            None,
        )
        .expect("managed runtime");

        assert_eq!(runtime.fd_required(), 128);
        assert!(!runtime.started());
        assert!(!runtime.stopped());
        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        tokio_runtime.block_on(async { runtime.start().expect("start") });
        assert!(runtime.started());
        runtime.stop();
        assert!(runtime.stopped());
        assert!(runtime.overlay().is_stopping());
        assert!(
            runtime
                .peerfinder_thread
                .lock()
                .expect("peerfinder worker lock")
                .is_none()
        );
    }

    #[test]
    fn overlay_tls_helpers_build_anonymous_client_and_server_configs() {
        assert!(default_overlay_client_config().is_ok());
        assert!(
            overlay_server_config(&crate::ServerPortOverlaySetup {
                ip: "0.0.0.0".to_owned(),
                port: 51235,
                limit: 64,
                secure: true,
                ssl_key: String::new(),
                ssl_cert: String::new(),
                ssl_chain: String::new(),
                ssl_ciphers: String::new(),
            })
            .expect("server config")
            .is_some()
        );
    }

    #[test]
    fn peer_limit_configuration_matches_rippled_legacy_and_directional_rules() {
        let defaulted = build_overlay_setup(&config("")).expect("default setup");
        assert_eq!(defaulted.peer_limits().max_peers, 21);
        assert_eq!(defaulted.peer_limits().inbound_max, 11);
        assert_eq!(defaulted.peer_limits().outbound_max, 10);

        // rippled raises a legacy max below kMinOutCount to ten before
        // deriving in/out maxima.
        let legacy = build_overlay_setup(&config("[peers_max]\n8\n")).expect("legacy setup");
        assert_eq!(legacy.peer_limits().max_peers, 10);
        assert_eq!(legacy.peer_limits().inbound_max, 0);
        assert_eq!(legacy.peer_limits().outbound_max, 10);

        let explicit = build_overlay_setup(&config("[peers_in_max]\n7\n[peers_out_max]\n10\n"))
            .expect("explicit setup");
        // rippled Config.cpp:112: config.maxPeers = 0 when explicit in/out set
        assert_eq!(explicit.peer_limits().max_peers, 0);
        assert_eq!(explicit.peer_limits().inbound_max, 7);
        assert_eq!(explicit.peer_limits().outbound_max, 10);

        assert_eq!(
            build_overlay_setup(&config("[peers_in_max]\n7\n"))
                .err()
                .expect("incomplete directional limits must fail"),
            "Both [peers_in_max] and [peers_out_max] must be configured"
        );
        assert_eq!(
            build_overlay_setup(&config("[peers_in_max]\n7\n[peers_out_max]\n9\n"))
                .err()
                .expect("outbound limit below ten must fail"),
            "Outbound peer limit must be in the range 10-1000"
        );
    }

    #[test]
    fn peerfinder_outbound_target_percent_and_minimum_shape() {
        assert_eq!(peerfinder_outbound_target(21, true), 10);
        assert_eq!(peerfinder_outbound_target(64, true), 10);
        assert_eq!(peerfinder_outbound_target(100, true), 15);
        assert_eq!(peerfinder_outbound_target(8, true), 10);
        assert_eq!(peerfinder_outbound_target(21, false), 21);
    }

    #[test]
    fn bootstrap_bootcache_stage_runs_while_outbound_is_below_target() {
        assert!(bootstrap_needs_bootcache_dial(0, 10));
        assert!(bootstrap_needs_bootcache_dial(1, 10));
        assert!(bootstrap_needs_bootcache_dial(9, 10));
        assert!(!bootstrap_needs_bootcache_dial(10, 10));
    }

    #[test]
    fn bootstrap_bootcache_stage_allows_parallel_pending_attempts() {
        assert!(bootstrap_can_dial_bootcache(true, 5, 10));
        assert!(!bootstrap_can_dial_bootcache(true, 10, 10));
        assert!(!bootstrap_can_dial_bootcache(false, 0, 10));
    }

    #[test]
    fn peerfinder_autoconnect_matches_reference_overlay_timer_cadence() {
        // Local rippled OverlayImpl::Timer::onTimer invokes autoConnect once
        // per one-second timer firing; the unused tuning declaration does not
        // introduce a ten-second scheduler gate.
        assert_eq!(PEERFINDER_AUTOCONNECT_INTERVAL, Duration::from_secs(1));
    }

    #[test]
    fn peerfinder_attempt_budget_is_global_not_outbound_target_limited() {
        // PeerFinder::Counts::attemptsNeeded only subtracts active connect
        // attempts from kMaxConnectAttempts. It does not subtract the normal
        // outbound target deficit, because fixed slots are exempt and normal
        // attempts race to activation admission.
        assert_eq!(peerfinder_attempt_budget(0), 20);
        assert_eq!(peerfinder_attempt_budget(19), 1);
        assert_eq!(peerfinder_attempt_budget(20), 0);
        assert_eq!(peerfinder_attempt_budget(21), 0);
    }

    #[test]
    fn fixed_stage_blocks_live_and_bootcache_until_its_batch_resolves() {
        // Logic::autoconnect returns after emitting fixed endpoints, and also
        // returns if no fixed endpoint is eligible while an attempt exists.
        assert!(fixed_stage_blocks_automatic_dials(0, 2, 1, 1));
        assert!(fixed_stage_blocks_automatic_dials(0, 2, 0, 1));
        assert!(!fixed_stage_blocks_automatic_dials(0, 2, 0, 0));
        assert!(!fixed_stage_blocks_automatic_dials(2, 2, 0, 1));
    }

    #[test]
    fn livecache_stage_blocks_bootcache_fallback_until_attempts_resolve() {
        // Logic::autoconnect returns after a livecache handout, and waits on
        // an existing outbound attempt when no livecache address is eligible.
        assert!(livecache_stage_blocks_bootcache(1, 1));
        assert!(livecache_stage_blocks_bootcache(0, 1));
        assert!(!livecache_stage_blocks_bootcache(0, 0));
    }

    #[test]
    fn peer_private_disables_automatic_bootcache_dials_but_keeps_fixed_peers() {
        let configured = config(
            "[peer_private]\n1\n[ips_fixed]\nfixed.example.com 51236\n[ips]\nbootstrap.example.com 51235\n",
        );
        let runtime = build_overlay_runtime(
            &configured,
            None,
            Arc::new(BootstrapOverlayHandoff),
            None,
            None,
        )
        .expect("runtime");

        // rippled PeerFinder::Config.cpp: autoConnect = !standalone && !peerPrivate.
        assert!(!runtime.bootstrap_can_dial_bootcache());
        assert_eq!(
            runtime.fixed_peer_endpoints,
            vec!["fixed.example.com:51236".to_owned()]
        );
    }

    #[test]
    fn mixed_peer_http_ws_port_is_owned_by_server_runtime_once() {
        let configured = config(
            "[server]\nport_mixed\n\n[port_mixed]\nip = 127.0.0.1\nport = 51235\nprotocol = peer,http,ws\nlimit = 64\n",
        );
        let ports = crate::ServerPortsSetup::from_config(&configured, false).expect("server ports");
        let runtime = build_overlay_runtime(
            &configured,
            Some(&ports),
            Arc::new(BootstrapOverlayHandoff),
            None,
            None,
        )
        .expect("runtime");

        assert!(runtime.server_owns_listener());
        assert_eq!(runtime.fd_required(), 0);
        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        tokio_runtime.block_on(async { runtime.start().expect("start") });
        runtime.stop();
    }

    #[test]
    fn fixed_peer_retry_state_is_due_on_first_cycle_fixed() {
        let endpoint = "203.0.113.50:51235".parse().expect("endpoint");
        let now = Instant::now();
        let state = HashMap::new();

        let retry_state = fixed_retry_state_or_due(&state, endpoint, now);

        assert_eq!(retry_state.0, 0);
        assert_eq!(retry_state.1, now);
        assert!(retry_state.1 <= now);
    }

    #[test]
    fn peerfinder_dns_cache_spawns_one_resolve_and_holds_in_flight() {
        let endpoint = "seed.example.com:51235";
        let now = Instant::now();
        let mut cache = PeerfinderDnsCache::new();

        assert!(cache.refresh(endpoint, now));
        assert!(!cache.refresh(endpoint, now));
        assert!(cache.addresses(endpoint).is_none());
    }

    #[test]
    fn peerfinder_dns_success_serves_addresses_and_re_resolves_after_ttl() {
        let endpoint = "seed.example.com:51235";
        let addresses = vec!["203.0.113.7:51235".parse().expect("address")];
        let now = Instant::now();
        let mut cache = PeerfinderDnsCache::new();

        assert!(cache.refresh(endpoint, now));
        cache.record(endpoint.to_owned(), Ok(addresses.clone()), now);
        assert!(!cache.refresh(endpoint, now));
        assert_eq!(cache.addresses(endpoint), Some(addresses.as_slice()));

        let after_ttl = now + PEERFINDER_DNS_TTL + Duration::from_secs(1);
        assert!(cache.refresh(endpoint, after_ttl));
    }

    #[test]
    fn peerfinder_dns_failure_keeps_last_good_addresses_and_backs_off() {
        let endpoint = "seed.example.com:51235";
        let addresses = vec!["203.0.113.7:51235".parse().expect("address")];
        let now = Instant::now();
        let mut cache = PeerfinderDnsCache::new();

        cache.record(endpoint.to_owned(), Ok(addresses.clone()), now);
        assert_eq!(cache.addresses(endpoint), Some(addresses.as_slice()));

        let failed_at = now + PEERFINDER_DNS_TTL;
        cache.record(
            endpoint.to_owned(),
            Err("DNS resolution timed out".to_owned()),
            failed_at,
        );
        assert_eq!(cache.addresses(endpoint), Some(addresses.as_slice()));
        assert!(!cache.refresh(
            endpoint,
            failed_at + PEERFINDER_DNS_FAILURE_COOLDOWN - Duration::from_secs(1)
        ));
        assert!(cache.refresh(endpoint, failed_at + PEERFINDER_DNS_FAILURE_COOLDOWN));
    }

    #[test]
    fn peerfinder_dns_background_resolve_round_trips_through_the_tick() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async {
            let (dns_tx, mut dns_rx) = tokio::sync::mpsc::unbounded_channel();
            let semaphore = Arc::new(tokio::sync::Semaphore::new(PEERFINDER_DNS_MAX_CONCURRENT));
            let endpoint = "127.0.0.1:51235".to_owned();
            spawn_peerfinder_dns_resolve(endpoint.clone(), dns_tx, semaphore);

            let (resolved, outcome) = dns_rx.recv().await.expect("resolve result");
            assert_eq!(resolved, endpoint);
            let addresses = outcome.expect("IP literal resolves");
            assert_eq!(addresses, vec!["127.0.0.1:51235".parse().expect("address")]);

            let now = Instant::now();
            let mut cache = PeerfinderDnsCache::new();
            cache.record(endpoint.clone(), Ok(addresses.clone()), now);
            assert_eq!(cache.addresses(&endpoint), Some(addresses.as_slice()));
        });
    }

    #[test]
    fn bootstrap_peer_endpoints_follow_cpp_ips_fixed_fallback_order() {
        let configured = config("[ips]\nseed.example.com 6000\n[ips_fixed]\nfixed.example.com\n");
        let fixed = parse_peer_endpoints(&configured, "ips_fixed").expect("fixed endpoints");
        assert_eq!(
            parse_bootstrap_peer_endpoints(&configured, &fixed).expect("bootstrap endpoints"),
            vec!["seed.example.com:6000".to_owned()]
        );

        let fixed_only = config("[ips_fixed]\nfixed.example.com\n");
        let fixed_only_endpoints =
            parse_peer_endpoints(&fixed_only, "ips_fixed").expect("fixed-only endpoints");
        assert_eq!(
            parse_bootstrap_peer_endpoints(&fixed_only, &fixed_only_endpoints)
                .expect("bootstrap fallback"),
            vec!["fixed.example.com:51235".to_owned()]
        );

        let defaults = parse_bootstrap_peer_endpoints(&config(""), &[]).expect("defaults");
        assert_eq!(defaults.len(), 4);
        assert_eq!(defaults[0], "r.ripple.com:51235");
    }

    #[test]
    fn fixed_peer_ips_parse_ipv6_and_canonicalize_mapped_ipv4() {
        let fixed = parse_peer_endpoints(
            &config("[ips_fixed]\n[2001:4860:4860::8888] 51235\n[::ffff:203.0.113.7] 51235\n"),
            "ips_fixed",
        )
        .expect("fixed endpoints");

        let ips = parse_fixed_peer_ips(&fixed);

        assert!(ips.contains(&IpAddr::V6(
            "2001:4860:4860::8888".parse::<Ipv6Addr>().expect("ipv6")
        )));
        assert!(ips.contains(&IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7))));
    }

    #[test]
    fn bootcache_static_entries_keep_cpp_static_valence() {
        let endpoint = "203.0.113.10:51235".parse().expect("endpoint");
        let mut bootcache = HashMap::new();

        remember_bootcache_endpoint(&mut bootcache, endpoint, false);
        bootcache_on_failure(&mut bootcache, endpoint);
        remember_bootcache_endpoint(&mut bootcache, endpoint, true);

        assert_eq!(
            bootcache.get(&endpoint).expect("bootcache entry").valence,
            BOOTCACHE_STATIC_VALENCE
        );
    }

    #[test]
    fn bootcache_valence_tracks_success_and_failure_streaks() {
        let endpoint = "203.0.113.11:51235".parse().expect("endpoint");
        let mut bootcache = HashMap::new();

        bootcache_on_success(&mut bootcache, endpoint);
        bootcache_on_success(&mut bootcache, endpoint);
        assert_eq!(bootcache.get(&endpoint).expect("success entry").valence, 2);

        bootcache_on_failure(&mut bootcache, endpoint);
        assert_eq!(bootcache.get(&endpoint).expect("failure entry").valence, -1);
    }

    #[test]
    fn persistent_live_bootcache_reloads_and_ranks_failed_endpoints_last() {
        let dir = tempfile::TempDir::new().expect("peerfinder cache dir");
        let path = dir.path().join("peerfinder.db");
        let preferred: std::net::SocketAddr = "8.8.8.8:51235".parse().expect("preferred");
        let fresh: std::net::SocketAddr = "1.1.1.1:51235".parse().expect("fresh");
        let failed: std::net::SocketAddr = "9.9.9.9:51235".parse().expect("failed");
        let now = Instant::now();
        let mut initial = PeerfinderCaches::loaded(Vec::new(), now);
        initial.on_success(preferred);
        initial.on_success(preferred);
        initial.insert_bootcache(fresh);
        initial.on_success(failed);
        initial.on_failure(failed);

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        assert!(runtime.block_on(persist_peerfinder_bootcache(
            Some(path.clone()),
            initial.entries(),
        )));

        // Recreate the live cache owner exactly as AppOverlayRuntime does at
        // startup, rather than reusing the in-memory map from the prior run.
        let recreated = runtime.block_on(load_peerfinder_caches(Some(path)));
        assert_eq!(
            recreated.bootcache.get(&preferred),
            Some(&BootcacheEntry { valence: 2 })
        );
        assert_eq!(
            recreated.bootcache.get(&fresh),
            Some(&BootcacheEntry { valence: 0 })
        );
        assert_eq!(
            recreated.bootcache.get(&failed),
            Some(&BootcacheEntry { valence: -1 })
        );

        // `Bootcache::onFailure` resets a formerly successful endpoint to
        // -1; bootcache iteration is decreasing valence, so it follows zero
        // and positive entries unless a recent-attempt squelch excludes it.
        let selected = select_bootcache_endpoints(
            &BTreeSet::new(),
            &recreated.bootcache,
            &HashMap::new(),
            Instant::now(),
            3,
        );
        assert_eq!(selected, vec![preferred, fresh, failed]);
    }

    #[test]
    fn select_bootcache_endpoints_prefers_high_valence_and_skips_recent_ips() {
        let mut bootcache = HashMap::new();
        let preferred: std::net::SocketAddr = "203.0.113.21:51235".parse().expect("preferred");
        let same_ip_other_port: std::net::SocketAddr =
            "203.0.113.21:6000".parse().expect("same ip");
        let fresh: std::net::SocketAddr = "203.0.113.22:51235".parse().expect("fresh");
        let recent: std::net::SocketAddr = "203.0.113.23:51235".parse().expect("recent");
        let now = Instant::now();

        bootcache.insert(preferred, BootcacheEntry { valence: 5 });
        bootcache.insert(same_ip_other_port, BootcacheEntry { valence: 4 });
        bootcache.insert(fresh, BootcacheEntry { valence: 3 });
        bootcache.insert(recent, BootcacheEntry { valence: 6 });

        let connected = BTreeSet::new();
        let recent_attempts = HashMap::from([(recent.ip(), now + Duration::from_secs(30))]);
        let selected = select_bootcache_endpoints(&connected, &bootcache, &recent_attempts, now, 3);

        assert_eq!(selected, vec![preferred, fresh]);
    }
}
