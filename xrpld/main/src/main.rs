use app::{
    AppBootstrapOptions, AppBootstrapRuntime, MainRuntime, ManagedComponent,
    RclValidationAcceptanceSink, build_bootstrap_root, load_basic_config_file,
    parse_bootstrap_args, run_bootstrap_runtime,
};
use basics::base_uint::Uint256;
use basics::basic_config::BasicConfig;
use overlay::Overlay;
use overlay::Peer as _;
// Import PeerSet trait for method access on SimplePeerSet
use overlay::PeerSet as _;
use protocol::{STValidation, SerialIter};
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

fn acq_packet_debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("XRPLD_ACQ_PACKET_DEBUG")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    })
}

fn get_ledger_request_shape(msg: &overlay::ProtocolMessage) -> (i32, usize, Option<u32>) {
    match &msg.payload {
        overlay::ProtocolPayload::GetLedger(request) => {
            (request.itype, request.node_i_ds.len(), request.query_depth)
        }
        _ => (0, 0, None),
    }
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

fn should_process_acquisition_tick(
    has_queued_data: bool,
    timer_due: bool,
    fetch_pack_ready: bool,
    first_add_peers: bool,
    peers_updated: bool,
) -> bool {
    has_queued_data || timer_due || fetch_pack_ready || (first_add_peers && peers_updated)
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

    eprintln!("    Run `xrpld --help` to see available commands.");
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

    tracing::info!(target: "main", version = env!("CARGO_PKG_VERSION"), "XRPLD starting");

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

fn hash_for_seq_from_completed_inbound_ledgers(
    inbound_ledgers: &InboundLedgers,
    target_seq: u32,
) -> Option<basics::sha_map_hash::SHAMapHash> {
    inbound_ledgers
        .entries
        .values()
        .filter_map(|entry| match &entry.state {
            InboundState::Complete(ledger) if ledger.header().seq >= target_seq => Some(ledger),
            _ => None,
        })
        .min_by_key(|ledger| ledger.header().seq)
        .and_then(|ledger| hash_for_seq_from_reference_ledger(ledger, target_seq))
}

fn hash_for_seq_from_available_sources(
    target_seq: u32,
    inbound_ledgers: &InboundLedgers,
    history_ledger: Option<&std::sync::Arc<ledger::Ledger>>,
    validated_ledger: Option<&std::sync::Arc<ledger::Ledger>>,
    loaded_runtime: Option<&app::AppLoadedLedgerRuntime>,
) -> Option<basics::sha_map_hash::SHAMapHash> {
    hash_for_seq_from_completed_inbound_ledgers(inbound_ledgers, target_seq)
        .or_else(|| {
            history_ledger.and_then(|ledger| hash_for_seq_from_reference_ledger(ledger, target_seq))
        })
        .or_else(|| {
            validated_ledger
                .and_then(|ledger| hash_for_seq_from_reference_ledger(ledger, target_seq))
        })
        .or_else(|| loaded_runtime.and_then(|runtime| runtime.get_hash_by_index(target_seq)))
}

fn candidate_reference_hash_from_completed_inbound_ledgers(
    inbound_ledgers: &InboundLedgers,
    target_seq: u32,
) -> Option<(u32, basics::sha_map_hash::SHAMapHash)> {
    inbound_ledgers
        .entries
        .values()
        .filter_map(|entry| match &entry.state {
            InboundState::Complete(ledger) if ledger.header().seq >= target_seq => Some(ledger),
            _ => None,
        })
        .min_by_key(|ledger| ledger.header().seq)
        .and_then(|ledger| candidate_reference_hash_from_reference_ledger(ledger, target_seq))
}

fn candidate_reference_hash_from_available_sources(
    target_seq: u32,
    inbound_ledgers: &InboundLedgers,
    history_ledger: Option<&std::sync::Arc<ledger::Ledger>>,
    validated_ledger: Option<&std::sync::Arc<ledger::Ledger>>,
    loaded_runtime: Option<&app::AppLoadedLedgerRuntime>,
) -> Option<(u32, basics::sha_map_hash::SHAMapHash)> {
    candidate_reference_hash_from_completed_inbound_ledgers(inbound_ledgers, target_seq)
        .or_else(|| {
            history_ledger.and_then(|ledger| {
                candidate_reference_hash_from_reference_ledger(ledger, target_seq)
            })
        })
        .or_else(|| {
            validated_ledger.and_then(|ledger| {
                candidate_reference_hash_from_reference_ledger(ledger, target_seq)
            })
        })
        .or_else(|| {
            let candidate_seq = candidate_ledger_for_seq(target_seq);
            (candidate_seq > target_seq)
                .then(|| {
                    loaded_runtime
                        .and_then(|runtime| runtime.get_hash_by_index(candidate_seq))
                        .map(|hash| (candidate_seq, hash))
                })
                .flatten()
        })
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
#[allow(dead_code)]
const MAX_LEDGER_GAP_TO_PUBLISH_SEQUENTIALLY: u32 = 100;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LedgerPublishAdvance {
    FirstPublished,
    GapTooLarge,
    Sequential,
    NothingToPublish,
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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

fn advance_published_ledgers_after_validation(
    app: &app::ApplicationRoot,
    peers: &[Arc<dyn overlay::Peer>],
    inbound_ledgers: &mut InboundLedgers,
    validated_ledger: std::sync::Arc<ledger::Ledger>,
) -> usize {
    app.note_validated_ledger_for_sync(std::sync::Arc::clone(&validated_ledger));
    let Some(lm_rt) = app.ledger_master_runtime() else {
        promote_current_ledger(app, peers, validated_ledger);
        return 1;
    };
    let ledger_master = lm_rt.ledger_master();
    // Ensure the runtime's LedgerMaster knows the validated ledger
    // (reference has a single LedgerMaster; we have two state holders)
    // Use no_sweep: during catchup the ledger's state map may not be
    // fully traversable (only acquired delta nodes are loaded).
    let mut vl = app.ledger_with_node_fetcher(std::sync::Arc::clone(&validated_ledger));
    {
        let l = std::sync::Arc::make_mut(&mut vl);
        l.set_validated();
        l.set_full();
        l.finalize_immutable_no_setup();
    }
    ledger_master.set_valid_ledger_no_sweep(std::sync::Arc::clone(&vl), None, None);

    // tryAdvance burst: promote consecutive ledgers in history that have
    // sufficient validations (matching rippled LedgerMaster::doAdvance loop).
    // This allows advancing N+1, N+2, ... N+50 in one tick when history
    // ledgers arrived before their validations were counted.
    {
        let needed = if app.standalone() {
            0
        } else {
            app.validators().quorum()
        };
        let mut burst_count = 0u32;
        loop {
            let next_seq = ledger_master.valid_ledger_seq() + 1;
            let Some(candidate) = ledger_master
                .ledger_history()
                .get_cached_ledger_by_seq(next_seq)
            else {
                break;
            };
            let candidate_hash = *candidate.header().hash.as_uint256();
            let validations = app
                .validations()
                .store()
                .trusted_for_ledger_by_sequence(candidate_hash, next_seq);
            let val_count = app
                .validators()
                .negative_unl_filter_validations(validations)
                .len();
            if val_count < needed {
                break;
            }
            // Promote to validated
            let mut next_vl = app.ledger_with_node_fetcher(std::sync::Arc::clone(&candidate));
            {
                let l = std::sync::Arc::make_mut(&mut next_vl);
                l.set_validated();
                l.set_full();
                l.finalize_immutable_no_setup();
            }
            ledger_master
                .ledger_history()
                .insert(std::sync::Arc::clone(&next_vl), true);
            ledger_master.mark_ledger_complete(next_seq);
            ledger_master.set_valid_ledger_no_sweep(std::sync::Arc::clone(&next_vl), None, None);
            app.note_validated_ledger_for_sync(std::sync::Arc::clone(&next_vl));
            burst_count += 1;
        }
        if burst_count > 0 {
            tracing::info!(target: "ledger_master",
                burst_count,
                new_valid_seq = ledger_master.valid_ledger_seq(),
                "tryAdvance burst: validated consecutive ledgers from history"
            );
        }
    }

    let report = lm_rt.plan_advance_publication();

    tracing::debug!(target: "ledger_master",
        decision = ?report.decision,
        valid_seq = validated_ledger.header().seq,
        published_seq = app.published_ledger_seq().unwrap_or(0),
        "Advance publication"
    );

    if let Some(missing) = report.missing {
        inbound_ledgers.acquire(missing.hash, missing.seq, ledger_master.valid_ledger_seq());
        tracing::debug!(target: "ledger_master",
            seq = missing.seq,
            valid_seq = ledger_master.valid_ledger_seq(),
            "Waiting for sequential ledger"
        );
    }

    let mut published = 0;
    for publish_ledger in report.published {
        let mut ledger = app.ledger_with_node_fetcher(std::sync::Arc::clone(&publish_ledger));
        {
            let l = std::sync::Arc::make_mut(&mut ledger);
            l.set_validated();
            l.set_full();
            l.finalize_immutable_no_setup();
        }
        ledger_master.mark_ledger_complete(ledger.header().seq);
        ledger_master.set_pub_ledger(std::sync::Arc::clone(&ledger));
        promote_current_ledger(app, peers, std::sync::Arc::clone(&ledger));
        published += 1;
    }

    // ledger so consensus can build on it. If plan_advance_publication
    // returned NothingToPublish (because try_promote already set pub_ledger),
    // we still need to promote so the app-level current ledger is updated.
    if published == 0 {
        promote_current_ledger(app, peers, vl);
    }

    rpc::update_validated_snapshot_cache(app);

    published.max(1)
}

///
/// After inserting acquired ledgers into history, walk pub_seq+1 → val_seq
/// sequentially. For each seq, look up the ledger in history by hash (using
/// the validated ledger's skip list), then build it using the previous ledger
/// as parent. This guarantees the parent is always available before the child
/// is built — exactly how reference processes ledgers.
///
/// Pure acquire-and-trust: walk pub_seq+1 → val_seq sequentially.
/// For each seq, look up the peer-acquired ledger in history by hash
/// (using the validated ledger's skip list), mark it as validated/full,
/// and set it as pub_ledger. No transaction replay — matches reference exactly.
fn try_advance_catchup(
    app: &app::ApplicationRoot,
    _node_store: Option<&app::SHAMapStoreNodeStore>,
    peers: &[std::sync::Arc<dyn overlay::Peer>],
    inbound_ledgers: &mut InboundLedgers,
) {
    let Some(lm_rt) = app.ledger_master_runtime() else {
        return;
    };
    let ledger_master = lm_rt.ledger_master();

    // Match rippled findNewLedgersToPublish: acquire up to LEDGER_FETCH_SIZE
    // sequential missing ledgers, not just one.
    const LEDGER_FETCH_SIZE: u32 = 4;

    let report = lm_rt.plan_advance_publication();
    if let Some(missing) = report.missing {
        inbound_ledgers.acquire(missing.hash, missing.seq, ledger_master.valid_ledger_seq());

        // Acquire additional sequential ledgers beyond the first missing one
        // (matching rippled's acqCount < ledgerFetchSize_ loop)
        if let Some(validated) = ledger_master.validated_ledger() {
            let valid_seq = validated.header().seq;
            let mut acquired = 1u32;
            let mut seq = missing.seq + 1;
            while acquired < LEDGER_FETCH_SIZE && seq <= valid_seq {
                if let Some(hash) = validated.hash_of_seq(seq, &ledger::NullLedgerJournal)
                    && !hash.is_zero() {
                        inbound_ledgers.acquire(
                            *hash.as_uint256(),
                            seq,
                            ledger_master.valid_ledger_seq(),
                        );
                        acquired += 1;
                    }
                seq += 1;
            }
        }
    }

    for publish_ledger in report.published {
        let mut ledger = app.ledger_with_node_fetcher(publish_ledger);
        {
            let l = std::sync::Arc::make_mut(&mut ledger);
            l.set_validated();
            l.set_full();
            l.finalize_immutable_no_setup();
        }
        ledger_master.mark_ledger_complete(ledger.header().seq);
        ledger_master.set_pub_ledger(std::sync::Arc::clone(&ledger));
        promote_current_ledger(app, peers, std::sync::Arc::clone(&ledger));
        tracing::info!(target: "catchup", seq = ledger.header().seq, "Locally built ledger — hash verified ✓");
    }
}

fn try_promote_ledger_with_validations(
    app: &app::ApplicationRoot,
    ledger_master: &app::AppLedgerMaster,
    ledger: std::sync::Arc<ledger::Ledger>,
    validation_count: usize,
    needed_validations: usize,
    allow_direct_peer_accept: bool,
) -> bool {
    let check = ledger_master.check_accept_ledger(
        ledger.as_ref(),
        validation_count,
        needed_validations,
        app.current_close_time_seconds(),
    );
    if check {
        let mut ledger = app.ledger_with_node_fetcher(ledger);
        {
            let l = std::sync::Arc::make_mut(&mut ledger);
            l.set_validated();
            l.set_full();
            // Mark immutable without setup_from_state_map (which traverses
            // the state map and may block on missing nodes during catchup).
            // finalize_immutable(false) just sets the flag.
            l.finalize_immutable_no_setup();
        }
        ledger_master
            .ledger_history()
            .insert(std::sync::Arc::clone(&ledger), true);
        ledger_master.mark_ledger_complete(ledger.header().seq);
        // Use no_sweep: during catchup the acquired ledger's state map
        // only has delta nodes and cannot be fully traversed.
        ledger_master.set_valid_ledger_no_sweep(std::sync::Arc::clone(&ledger), None, None);
        tracing::info!(target: "ledger_master", seq = ledger.header().seq, valid_ledger_seq_after = ledger_master.valid_ledger_seq(), "set_valid");
        if ledger_master.published_ledger().is_none() {
            ledger_master.set_pub_ledger(std::sync::Arc::clone(&ledger));
        }
        // Persist validated ledger hash+seq for restart bootstrap
        let hash_hex: String = ledger
            .header()
            .hash
            .as_uint256()
            .data()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();
        let _ = std::fs::write(
            "/mnt/xrpl-data/testnet/last_validated.txt",
            format!("{} {}", hash_hex, ledger.header().seq),
        );
        // Keep the catchup promotion path limited to validated-ledger state so
        // tryAdvance-like progress is not blocked by SHAMap store side effects.
        app.note_validated_ledger_for_sync(std::sync::Arc::clone(&ledger));
        return true;
    } else {
        tracing::debug!(target: "ledger_master", seq = ledger.header().seq, val_count = validation_count, needed = needed_validations, valid_seq = ledger_master.valid_ledger_seq(), "check_accept=false");
    }

    if allow_direct_peer_accept {
        let persistence = ledger::LedgerPersistence::new(std::sync::Arc::new(
            app.build_ledger_persistence_runtime(),
        ));
        let ledger = app.ledger_with_node_fetcher(ledger);
        let _ = ledger_master.set_full_ledger(&persistence, ledger, true, true, None, None);
        return true;
    }

    false
}

#[allow(dead_code)]
fn should_attempt_completed_ledger_promotion(
    acquired_seq: u32,
    current_validated_seq: u32,
) -> bool {
    // behind validLedgerSeq_. They are useful history, not promotion candidates.
    acquired_seq > current_validated_seq
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletedLedgerAcceptance {
    HistoricalCached,
    HeldForQuorum,
    ValidatedAccepted,
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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
        .get_preferred(app::validated_ledger_from_ledger(
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
                .get_preferred(app::validated_ledger_from_ledger(
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
struct CheckAcceptSink {
    app: app::ApplicationRoot,
    validated_tx: std::sync::mpsc::Sender<(Uint256, u32)>,
}

impl RclValidationAcceptanceSink for CheckAcceptSink {
    fn check_accept(&self, hash: Uint256, seq: u32) {
        let valid_ledger_seq = self.app.validated_ledger_seq().unwrap_or(0);
        if seq != 0 && seq <= valid_ledger_seq {
            return;
        }
        let has_local = self
            .app
            .validated_ledger()
            .is_some_and(|l| *l.header().hash.as_uint256() == hash);
        if has_local {
            return;
        }

        // Immediate promotion: if we have the ledger in history AND enough
        // validations, promote to validated RIGHT NOW (matching rippled's
        // event-driven checkAccept that fires on each validation receipt).
        if let Some(lm_rt) = self.app.ledger_master_runtime() {
            let lm = lm_rt.ledger_master();
            if let Some(ledger) = lm.get_ledger_by_hash(
                basics::sha_map_hash::SHAMapHash::new(hash)
            ) {
                let validations = self.app.validations().store()
                    .trusted_for_ledger_by_sequence(hash, seq);
                let val_count = self.app.validators()
                    .negative_unl_filter_validations(validations).len();
                let needed = if self.app.standalone() { 0 } else { self.app.validators().quorum() };
                if val_count >= needed {
                    let mut promoted = self.app.ledger_with_node_fetcher(ledger);
                    {
                        let l = std::sync::Arc::make_mut(&mut promoted);
                        l.set_validated();
                        l.set_full();
                        l.finalize_immutable_no_setup();
                    }
                    lm.ledger_history().insert(std::sync::Arc::clone(&promoted), true);
                    lm.mark_ledger_complete(promoted.header().seq);
                    lm.set_valid_ledger_no_sweep(std::sync::Arc::clone(&promoted), None, None);
                    if lm.published_ledger().is_none() {
                        lm.set_pub_ledger(std::sync::Arc::clone(&promoted));
                    }
                }
            }
        }

        // Also send to the catchup loop for acquisition if we don't have it
        if seq != 0
            && valid_ledger_seq == 0
            && let Some(overlay) = self.app.overlay_runtime()
        {
            overlay.overlay().check_tracking(seq);
        }
        let _ = self.validated_tx.send((hash, seq));
    }
}

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

/// Process queued validations from the overlay and feed them into the
/// validation store, matching reference `PeerImp::checkValidation` →
/// `NetworkOPs::recvValidation` → `handleNewValidation` → `checkAccept`.
fn process_queued_validations(app: &app::ApplicationRoot, accept_sink: &CheckAcceptSink) {
    let Some(overlay_runtime) = app.overlay_runtime() else {
        return;
    };
    // Use take_validations to drain ONLY validations, leaving ledger_data
    // and get_objects for the ledger acquisition loop.
    let validations = overlay_runtime.overlay().take_validations();
    if validations.is_empty() {
        return;
    }
    for queued in &validations {
        let mut serial = SerialIter::new(&queued.message.validation);
        let parsed = STValidation::from_serial_iter_default_node_id(&mut serial, false);
        let mut validation = match parsed {
            Ok(v) => v,
            Err(_e) => {
                continue;
            }
        };
        // Set seen_time to now, matching reference which sets it on receipt.
        // Without this, seen_time=0 makes local_age check fail in is_current().
        let now_wall = app.current_close_time_seconds();
        validation.set_seen(now_wall);

        // Adjust our clock from the validator's sign_time so is_current() passes.
        // During cold bootstrap (need_network_ledger=true), allow unlimited offset
        // since our clock is at genesis time and the network is hours/days ahead.
        let sign_time = validation.get_sign_time();
        if sign_time > 0 {
            let now_wall_i = now_wall as i64;
            let sign_i = sign_time as i64;
            let offset = sign_i - now_wall_i;
            // Always trust validator sign_time for clock sync.
            // Validators are authoritative on network time.
            app.time_keeper()
                .adjust_close_time(time::Duration::seconds(offset));
        }
        let source = queued.peer_id.to_string();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            app.receive_validation_to_network_ops_with_accept(&mut validation, &source, accept_sink)
        }));
        let report = match result {
            Ok(r) => r,
            Err(e) => {
                let _msg = e
                    .downcast_ref::<String>()
                    .map(|s| s.as_str())
                    .or_else(|| e.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown");
                continue;
            }
        };
        if let Some(ref report) = report {
            tracing::debug!(target: "consensus", hash = %report.ledger_hash, trusted = report.trusted, current = report.current, bypass = report.bypass_accept, "Validation received");
        }
        // Relay trusted validations to other peers.
        if let Some(report) = &report
            && report.relay
        {
            overlay_runtime.overlay().relay_validation(
                queued.message.clone(),
                queued.suppression,
                *validation.get_signer_public(),
            );
        }
    }
}

// Helper types for persistent ledger acquisition
struct NodeStoreFetcher {
    node_store: app::SHAMapStoreNodeStore,
    pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
}

impl shamap::family::SHAMapNodeFetcher for NodeStoreFetcher {
    fn fetch_node_object(
        &self,
        hash: basics::sha_map_hash::SHAMapHash,
        ledger_seq: u32,
    ) -> Option<shamap::node_object::NodeObject> {
        if let Some(pending) = self
            .pending_writes
            .lock()
            .expect("pending node-store writes mutex")
            .get(hash.as_uint256())
            .cloned()
        {
            return Some(shamap::node_object::NodeObject::new(
                pending.shamap_type(),
                pending.data,
                pending.hash,
            ));
        }

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

#[derive(Debug, Clone)]
struct PendingNodeStoreObject {
    obj_type: nodestore::NodeObjectType,
    data: Vec<u8>,
    hash: Uint256,
}

impl PendingNodeStoreObject {
    fn shamap_type(&self) -> shamap::storage::NodeObjectType {
        match self.obj_type {
            nodestore::NodeObjectType::AccountNode => shamap::storage::NodeObjectType::AccountNode,
            nodestore::NodeObjectType::TransactionNode => {
                shamap::storage::NodeObjectType::TransactionNode
            }
            nodestore::NodeObjectType::Ledger => shamap::storage::NodeObjectType::Ledger,
            _ => shamap::storage::NodeObjectType::Unknown,
        }
    }
}

#[derive(Default)]
struct AcqCounters {
    ledger_data_packets: AtomicU64,
    run_data_batches: AtomicU64,
    run_data_packets: AtomicU64,
    ledger_headers_stored: AtomicU64,
    accepted_account_nodes: AtomicU64,
    accepted_transaction_nodes: AtomicU64,
    accepted_ledger_nodes: AtomicU64,
    accepted_unknown_nodes: AtomicU64,
    nodestore_writes_total: AtomicU64,
    nodestore_account_writes: AtomicU64,
    nodestore_transaction_writes: AtomicU64,
    nodestore_ledger_writes: AtomicU64,
    nodestore_unknown_writes: AtomicU64,
}

#[derive(Clone, Copy, Default)]
struct AcqPeerPacketStats {
    packets: u64,
    nodes: u64,
    inner_nodes: u64,
    leaf_nodes: u64,
    malformed_nodes: u64,
    useful: u64,
    invalid: u64,
    duplicate: u64,
    elapsed_ms: u64,
    last_request_nodes: usize,
}

impl AcqPeerPacketStats {
    fn record_request(&mut self, requested_nodes: usize) {
        self.last_request_nodes = requested_nodes;
    }

    fn record_packet(&mut self, stats: &ledger::InboundLedgerPacketDebugStats) {
        self.packets += 1;
        self.nodes += stats.shape.nodes as u64;
        self.inner_nodes += stats.shape.inner_nodes as u64;
        self.leaf_nodes += stats.shape.leaf_nodes as u64;
        self.malformed_nodes += stats.shape.malformed_nodes as u64;
        self.useful += stats.useful.max(0) as u64;
        self.invalid += stats.invalid.max(0) as u64;
        self.duplicate += stats.duplicate.max(0) as u64;
        self.elapsed_ms += stats.elapsed_ms as u64;
    }
}

#[derive(Clone, Copy, Default)]
struct AcqCountersSnapshot {
    ledger_data_packets: u64,
    run_data_batches: u64,
    run_data_packets: u64,
    ledger_headers_stored: u64,
    accepted_account_nodes: u64,
    accepted_transaction_nodes: u64,
    accepted_ledger_nodes: u64,
    accepted_unknown_nodes: u64,
    nodestore_writes_total: u64,
    nodestore_account_writes: u64,
    nodestore_transaction_writes: u64,
    nodestore_ledger_writes: u64,
    nodestore_unknown_writes: u64,
}

impl AcqCounters {
    fn inc_ledger_packet(&self) {
        self.ledger_data_packets.fetch_add(1, Ordering::Relaxed);
    }

    fn inc_run_data_batch(&self, processed_packets: usize) {
        self.run_data_batches.fetch_add(1, Ordering::Relaxed);
        self.run_data_packets
            .fetch_add(processed_packets as u64, Ordering::Relaxed);
    }

    fn inc_accepted_node(&self, obj_type: shamap::storage::NodeObjectType) {
        match obj_type {
            shamap::storage::NodeObjectType::AccountNode => {
                self.accepted_account_nodes.fetch_add(1, Ordering::Relaxed);
            }
            shamap::storage::NodeObjectType::TransactionNode => {
                self.accepted_transaction_nodes
                    .fetch_add(1, Ordering::Relaxed);
            }
            shamap::storage::NodeObjectType::Ledger => {
                self.accepted_ledger_nodes.fetch_add(1, Ordering::Relaxed);
            }
            shamap::storage::NodeObjectType::Unknown | shamap::storage::NodeObjectType::Dummy => {
                self.accepted_unknown_nodes.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn inc_nodestore_write(&self, obj_type: nodestore::NodeObjectType) {
        self.nodestore_writes_total.fetch_add(1, Ordering::Relaxed);
        match obj_type {
            nodestore::NodeObjectType::AccountNode => {
                self.nodestore_account_writes
                    .fetch_add(1, Ordering::Relaxed);
            }
            nodestore::NodeObjectType::TransactionNode => {
                self.nodestore_transaction_writes
                    .fetch_add(1, Ordering::Relaxed);
            }
            nodestore::NodeObjectType::Ledger => {
                self.nodestore_ledger_writes.fetch_add(1, Ordering::Relaxed);
            }
            _ => {
                self.nodestore_unknown_writes
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn inc_ledger_header_stored(&self) {
        self.ledger_headers_stored.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> AcqCountersSnapshot {
        AcqCountersSnapshot {
            ledger_data_packets: self.ledger_data_packets.load(Ordering::Relaxed),
            run_data_batches: self.run_data_batches.load(Ordering::Relaxed),
            run_data_packets: self.run_data_packets.load(Ordering::Relaxed),
            ledger_headers_stored: self.ledger_headers_stored.load(Ordering::Relaxed),
            accepted_account_nodes: self.accepted_account_nodes.load(Ordering::Relaxed),
            accepted_transaction_nodes: self.accepted_transaction_nodes.load(Ordering::Relaxed),
            accepted_ledger_nodes: self.accepted_ledger_nodes.load(Ordering::Relaxed),
            accepted_unknown_nodes: self.accepted_unknown_nodes.load(Ordering::Relaxed),
            nodestore_writes_total: self.nodestore_writes_total.load(Ordering::Relaxed),
            nodestore_account_writes: self.nodestore_account_writes.load(Ordering::Relaxed),
            nodestore_transaction_writes: self.nodestore_transaction_writes.load(Ordering::Relaxed),
            nodestore_ledger_writes: self.nodestore_ledger_writes.load(Ordering::Relaxed),
            nodestore_unknown_writes: self.nodestore_unknown_writes.load(Ordering::Relaxed),
        }
    }

    fn log_status(&self, seq: u32, stats: shamap::owners::sync::SHAMapAddNode, done: bool) {
        let snapshot = self.snapshot();
        tracing::debug!(target: "inbound_ledger",
            seq, done,
            packets = snapshot.ledger_data_packets,
            run_batches = snapshot.run_data_batches,
            run_packets = snapshot.run_data_packets,
            useful = stats.get_good(),
            invalid = stats.get_bad(),
            duplicate = stats.get_duplicate(),
            accepted_account = snapshot.accepted_account_nodes,
            accepted_tx = snapshot.accepted_transaction_nodes,
            accepted_ledger = snapshot.accepted_ledger_nodes,
            accepted_unknown = snapshot.accepted_unknown_nodes,
            writes_total = snapshot.nodestore_writes_total,
            writes_account = snapshot.nodestore_account_writes,
            writes_tx = snapshot.nodestore_transaction_writes,
            writes_ledger = snapshot.nodestore_ledger_writes,
            writes_unknown = snapshot.nodestore_unknown_writes,
            headers = snapshot.ledger_headers_stored,
            "Acquisition counters"
        );
    }
}

struct NodeStoreWriter {
    node_store: app::SHAMapStoreNodeStore,
    counters: Arc<AcqCounters>,
    write_tx: std::sync::mpsc::Sender<NodeStoreWriteMsg>,
    write_count: std::cell::Cell<u64>,
    pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
    /// Short-lived cross-acquisition write dedup. This avoids duplicate write
    /// bursts while acquisitions overlap, but it is bounded and swept so it
    /// does not retain every hash the node has ever written.
    shared_stored: Arc<basics::tagged_cache::KeyCache<Uint256>>,
}

enum NodeStoreWriteMsg {
    Write {
        obj_type: nodestore::NodeObjectType,
        data: Vec<u8>,
        hash: Uint256,
        seq: u32,
    },
    Flush(std::sync::mpsc::Sender<()>),
    #[allow(dead_code)]
    Stop,
}

fn flush_nodestore_writes(write_tx: &std::sync::mpsc::Sender<NodeStoreWriteMsg>) -> bool {
    let (ack_tx, ack_rx) = std::sync::mpsc::channel();
    if write_tx.send(NodeStoreWriteMsg::Flush(ack_tx)).is_err() {
        return false;
    }

    ack_rx.recv_timeout(Duration::from_secs(30)).is_ok()
}

/// Spawn a dedicated writer thread that drains writes from the channel.
/// This eliminates store_mutex contention from acquisition threads.
fn spawn_nodestore_writer(
    ns: app::SHAMapStoreNodeStore,
    pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
) -> (
    std::sync::mpsc::Sender<NodeStoreWriteMsg>,
    thread::JoinHandle<()>,
) {
    let (tx, rx) = std::sync::mpsc::channel::<NodeStoreWriteMsg>();
    let handle = thread::Builder::new()
        .name("xrpld-nudb-writer".to_owned())
        .spawn(move || {
            let mut total_writes = 0u64;
            let mut last_log = Instant::now();
            let do_store = |ns: &app::SHAMapStoreNodeStore, obj_type, data, hash, seq| match ns {
                app::SHAMapStoreNodeStore::Single(db) => db.store(obj_type, data, hash, seq),
                app::SHAMapStoreNodeStore::Rotating(db) => db.store(obj_type, data, hash, seq),
            };
            let mut total_store_us = 0u64;
            loop {
                // Block waiting for first message
                let first = match rx.recv() {
                    Ok(NodeStoreWriteMsg::Write {
                        obj_type,
                        data,
                        hash,
                        seq,
                    }) => Some((obj_type, data, hash, seq)),
                    Ok(NodeStoreWriteMsg::Flush(ack)) => {
                        let _ = ack.send(());
                        None
                    }
                    Ok(NodeStoreWriteMsg::Stop) | Err(_) => return,
                };
                // Process the first write
                if let Some((obj_type, data, hash, seq)) = first {
                    let t = Instant::now();
                    do_store(&ns, obj_type, data, hash, seq);
                    pending_writes
                        .lock()
                        .expect("pending node-store writes mutex")
                        .remove(&hash);
                    total_store_us += t.elapsed().as_micros() as u64;
                    total_writes += 1;
                }
                // Drain ALL queued writes without blocking
                loop {
                    match rx.try_recv() {
                        Ok(NodeStoreWriteMsg::Write {
                            obj_type,
                            data,
                            hash,
                            seq,
                        }) => {
                            let t = Instant::now();
                            do_store(&ns, obj_type, data, hash, seq);
                            pending_writes
                                .lock()
                                .expect("pending node-store writes mutex")
                                .remove(&hash);
                            total_store_us += t.elapsed().as_micros() as u64;
                            total_writes += 1;
                        }
                        Ok(NodeStoreWriteMsg::Flush(ack)) => {
                            let _ = ack.send(());
                        }
                        Ok(NodeStoreWriteMsg::Stop) => return,
                        Err(_) => break,
                    }
                }
                if last_log.elapsed() >= Duration::from_secs(10) {
                    let avg_us = if total_writes > 0 {
                        total_store_us / total_writes
                    } else {
                        0
                    };
                    tracing::debug!(target: "nodestore", total_writes, avg_us, "NuDB writer status");
                    last_log = Instant::now();
                }
            }
        })
        .expect("nudb writer thread");
    (tx, handle)
}

impl NodeStoreWriter {
    fn store_object(
        &mut self,
        obj_type: nodestore::NodeObjectType,
        data: Vec<u8>,
        hash: Uint256,
        seq: u32,
    ) {
        self.counters.inc_nodestore_write(obj_type);
        self.pending_writes
            .lock()
            .expect("pending node-store writes mutex")
            .insert(
                hash,
                PendingNodeStoreObject {
                    obj_type,
                    data: data.clone(),
                    hash,
                },
            );
        // Dedup already checked by should_store_hash before serialization
        let _ = self.write_tx.send(NodeStoreWriteMsg::Write {
            obj_type,
            data,
            hash,
            seq,
        });
        self.write_count.set(self.write_count.get() + 1);
    }
}

impl ledger::InboundLedgerStore for NodeStoreWriter {
    fn fetch_ledger_header(
        &mut self,
        hash: basics::sha_map_hash::SHAMapHash,
        _seq: u32,
    ) -> Option<Vec<u8>> {
        if let Some(pending) = self
            .pending_writes
            .lock()
            .expect("pending node-store writes mutex")
            .get(hash.as_uint256())
            .cloned()
        {
            return Some(pending.data);
        }

        let fetched = match &self.node_store {
            app::SHAMapStoreNodeStore::Single(db) => db.fetch_node_object(
                hash.as_uint256(),
                0,
                nodestore::FetchType::Synchronous,
                false,
            ),
            app::SHAMapStoreNodeStore::Rotating(db) => db.fetch_node_object(
                hash.as_uint256(),
                0,
                nodestore::FetchType::Synchronous,
                false,
            ),
        }?;
        Some(fetched.data().to_vec())
    }
    fn store_ledger_header(
        &mut self,
        data: Vec<u8>,
        hash: basics::sha_map_hash::SHAMapHash,
        seq: u32,
    ) {
        self.counters.inc_ledger_header_stored();
        self.store_object(
            nodestore::NodeObjectType::Ledger,
            data,
            *hash.as_uint256(),
            seq,
        );
    }
    fn store_shamap_node(
        &mut self,
        obj_type: shamap::storage::NodeObjectType,
        data: Vec<u8>,
        hash: Uint256,
        seq: u32,
    ) {
        self.counters.inc_accepted_node(obj_type);
        let mapped = match obj_type {
            shamap::storage::NodeObjectType::AccountNode => nodestore::NodeObjectType::AccountNode,
            shamap::storage::NodeObjectType::TransactionNode => {
                nodestore::NodeObjectType::TransactionNode
            }
            shamap::storage::NodeObjectType::Ledger => nodestore::NodeObjectType::Ledger,
            _ => nodestore::NodeObjectType::Unknown,
        };
        self.store_object(mapped, data, hash, seq);
    }

    fn should_store_hash(&mut self, hash: Uint256) -> bool {
        self.shared_stored.insert(hash)
    }

    fn fetch_node_data(&self, hash: Uint256) -> Option<basics::blob::Blob> {
        if let Some(pending) = self
            .pending_writes
            .lock()
            .expect("pending node-store writes mutex")
            .get(&hash)
            .cloned()
        {
            return Some(pending.data);
        }

        let fetched = match &self.node_store {
            app::SHAMapStoreNodeStore::Single(db) => {
                db.fetch_node_object(&hash, 0, nodestore::FetchType::Synchronous, false)
            }
            app::SHAMapStoreNodeStore::Rotating(db) => {
                db.fetch_node_object(&hash, 0, nodestore::FetchType::Synchronous, false)
            }
        }?;
        Some(fetched.data().to_vec())
    }
}

struct CatchupJournal;
impl ledger::InboundLedgerJournal for CatchupJournal {
    fn trace(&self, _msg: &str) { /* suppress trace in catchup */
    }
    fn debug(&self, msg: &str) {
        tracing::debug!(target: "catchup", "{msg}");
    }
    fn warn(&self, msg: &str) {
        tracing::warn!(target: "catchup", "{msg}");
    }
    fn fatal(&self, msg: &str) {
        tracing::error!(target: "catchup", "{msg}");
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

/// Result sent back from the processing thread to the main loop.
enum AcqResult {
    /// Ledger acquisition complete
    Complete(ledger::Ledger),
    /// Acquisition failed permanently
    Failed,
    /// Still in progress (periodic status)
    Progress {
        #[allow(dead_code)]
        good_nodes: usize,
    },
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

/// Limits concurrent run_data processing to match reference job queue thread count.
/// process data concurrently. This reduces cache mutex contention.
struct RunDataLimiter {
    state: Mutex<usize>,
    cv: std::sync::Condvar,
    max_concurrent: usize,
}

impl RunDataLimiter {
    fn new(max: usize) -> Self {
        Self {
            state: Mutex::new(0),
            cv: std::sync::Condvar::new(),
            max_concurrent: max,
        }
    }
    fn acquire(&self) {
        let mut count = self.state.lock().unwrap();
        while *count >= self.max_concurrent {
            count = self.cv.wait(count).unwrap();
        }
        *count += 1;
    }
    fn release(&self) {
        let mut count = self.state.lock().unwrap();
        *count -= 1;
        self.cv.notify_one();
    }
}

/// Deduplicates by hash, caches completed ledgers, sweeps inactive entries,
/// and starts each distinct requested ledger without a global active cap.
struct InboundLedgers {
    entries: HashMap<Uint256, InboundEntry>,
    recent_failures: HashMap<Uint256, Instant>,
    registry: AcqRegistry,
    // Shared resources for spawning acquisition threads
    node_store: Option<app::SHAMapStoreNodeStore>,
    tree_cache: Arc<shamap::tree_node_cache::TreeNodeCache<basics::tagged_cache::MonotonicClock>>,
    full_below: Arc<
        shamap::family::FullBelowCacheImpl<
            basics::tagged_cache::MonotonicClock,
            basics::hardened_hash::HardenedHashBuilder,
        >,
    >,
    fetch_pack: Arc<ledger::FetchPackCache>,
    write_tx: Option<std::sync::mpsc::Sender<NodeStoreWriteMsg>>,
    pending_writes: Option<Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>>,
    run_data_limiter: Arc<RunDataLimiter>,
    shared_stored: Arc<basics::tagged_cache::KeyCache<Uint256>>,
    /// Channel for workers to immediately notify of completed ledgers
    /// (matching rippled's done() → storeLedger() without polling delay).
    completed_ledgers_tx: std::sync::mpsc::Sender<Arc<ledger::Ledger>>,
    /// Overlay for sending initial peers to worker threads
    overlay_rt: Option<Arc<app::runtime::overlay_runtime::AppOverlayRuntime>>,
}

struct InboundEntry {
    seq: u32,
    tx: std::sync::mpsc::Sender<AcqMsg>,
    result_rx: std::sync::mpsc::Receiver<AcqResult>,
    #[allow(dead_code)]
    handle: thread::JoinHandle<()>,
    started_at: Instant,
    last_touched: Instant,
    completed_at: Option<Instant>,
    state: InboundState,
    skip_state: bool,
}

enum InboundState {
    InProgress,
    Complete(ledger::Ledger),
    #[allow(dead_code)]
    Failed,
}

impl InboundState {
    fn info_label(&self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Complete(_) => "complete",
            Self::Failed => "failed",
        }
    }
}

/// Reacquire interval for failed ledgers (reference kREACQUIRE_INTERVAL = 5 min)
const INBOUND_REACQUIRE_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// Sweep timeout for completed entries (reference 1 minute after last action)
const INBOUND_SWEEP_INTERVAL: Duration = Duration::from_secs(60);
/// Timeout for stuck InProgress entries (reference ~180s with no progress)
const INBOUND_STUCK_TIMEOUT: Duration = Duration::from_secs(180);

impl InboundLedgers {
    fn new(
        registry: AcqRegistry,
        tree_cache: Arc<
            shamap::tree_node_cache::TreeNodeCache<basics::tagged_cache::MonotonicClock>,
        >,
        full_below: Arc<
            shamap::family::FullBelowCacheImpl<
                basics::tagged_cache::MonotonicClock,
                basics::hardened_hash::HardenedHashBuilder,
            >,
        >,
        fetch_pack: Arc<ledger::FetchPackCache>,
        run_data_limiter: Arc<RunDataLimiter>,
        shared_stored: Arc<basics::tagged_cache::KeyCache<Uint256>>,
        completed_ledgers_tx: std::sync::mpsc::Sender<Arc<ledger::Ledger>>,
    ) -> Self {
        Self {
            entries: HashMap::new(),
            recent_failures: HashMap::new(),
            registry,
            node_store: None,
            tree_cache,
            full_below,
            fetch_pack,
            write_tx: None,
            pending_writes: None,
            run_data_limiter,
            shared_stored,
            completed_ledgers_tx,
            overlay_rt: None,
        }
    }

    fn set_overlay_rt(&mut self, rt: Arc<app::runtime::overlay_runtime::AppOverlayRuntime>) {
        self.overlay_rt = Some(rt);
    }

    fn set_node_store(&mut self, ns: app::SHAMapStoreNodeStore) {
        self.node_store = Some(ns);
    }

    fn set_write_tx(&mut self, tx: std::sync::mpsc::Sender<NodeStoreWriteMsg>) {
        self.write_tx = Some(tx);
    }

    fn set_pending_writes(
        &mut self,
        pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
    ) {
        self.pending_writes = Some(pending_writes);
    }

    /// Returns Some(ledger) if already complete, None if in-progress or newly started.
    fn acquire(&mut self, hash: Uint256, seq: u32, validated: u32) -> Option<&ledger::Ledger> {
        if hash.is_zero() {
            full_sync_debug!(
                "[full_debug][acq_request] reject seq={} hash={} reason=zero_or_genesis validated={}",
                seq,
                debug_hash8(&hash),
                validated
            );
            return None;
        }

        // Check recent failures (reference recentFailures aged_map)
        if let Some(failed_at) = self.recent_failures.get(&hash)
            && failed_at.elapsed() < INBOUND_REACQUIRE_INTERVAL
        {
            full_sync_debug!(
                "[full_debug][acq_request] reject seq={} hash={} reason=recent_failure age_ms={} validated={}",
                seq,
                debug_hash8(&hash),
                failed_at.elapsed().as_millis(),
                validated
            );
            return None;
        }
        self.recent_failures
            .retain(|_, t| t.elapsed() < INBOUND_REACQUIRE_INTERVAL);

        // If already tracked by hash, touch and return status
        if self.entries.contains_key(&hash) {
            let entry = self.entries.get_mut(&hash).unwrap();
            entry.last_touched = Instant::now();
            full_sync_debug!(
                "[full_debug][acq_request] existing seq={} hash={} state={} validated={}",
                seq,
                debug_hash8(&hash),
                entry.state.info_label(),
                validated
            );
            return match &entry.state {
                InboundState::Complete(ledger) => Some(ledger),
                InboundState::Failed => None,
                InboundState::InProgress => None,
            };
        }

        // Spawn new acquisition
        let Some(ns) = self.node_store.as_ref() else {
            full_sync_debug!(
                "[full_debug][acq_request] reject seq={} hash={} reason=no_node_store validated={}",
                seq,
                debug_hash8(&hash),
                validated
            );
            return None;
        };
        let Some(wt) = self.write_tx.as_ref() else {
            full_sync_debug!(
                "[full_debug][acq_request] reject seq={} hash={} reason=no_write_tx validated={}",
                seq,
                debug_hash8(&hash),
                validated
            );
            return None;
        };

        let shamap_hash = basics::sha_map_hash::SHAMapHash::new(hash);
        let (acq_tx, acq_rx) = std::sync::mpsc::channel::<AcqMsg>();
        let (acq_result_tx, acq_result_rx) = std::sync::mpsc::channel::<AcqResult>();
        let ns_clone = ns.clone();
        let tc = Arc::clone(&self.tree_cache);
        let fb = Arc::clone(&self.full_below);
        let fp = Arc::clone(&self.fetch_pack);
        let wt_clone = wt.clone();
        let pending_clone = self
            .pending_writes
            .as_ref()
            .expect("pending node-store writes must be configured before acquire")
            .clone();
        let rl = Arc::clone(&self.run_data_limiter);
        let ss = Arc::clone(&self.shared_stored);
        // skip_state optimization is not compatibility and causes tip ledgers
        // to appear done without being useful for the publish stream.
        let skip_state = false;
        let store_tx = self.completed_ledgers_tx.clone();

        let acq_handle = thread::Builder::new()
            .name("xrpld-acq-process".to_owned())
            .spawn(move || {
                run_acquisition_thread(
                    acq_rx,
                    acq_result_tx,
                    shamap_hash,
                    seq,
                    ns_clone,
                    tc,
                    fb,
                    fp,
                    wt_clone,
                    pending_clone,
                    rl,
                    ss,
                    skip_state,
                    store_tx,
                );
            })
            .expect("acquisition thread should spawn");

        // Register in overlay router
        self.registry
            .lock()
            .expect("acq registry")
            .insert(hash, acq_tx.clone());

        let now = Instant::now();

        // Send initial peers immediately so the worker has them on first trigger
        // (matching rippled InboundLedger::init → addPeers before first timer)
        if let Some(overlay_rt) = &self.overlay_rt {
            use overlay::Overlay as _;
            let peers = overlay_rt.overlay().active_peers();
            let _ = acq_tx.send(AcqMsg::Peers(peers));
        }

        self.entries.insert(
            hash,
            InboundEntry {
                seq,
                tx: acq_tx,
                result_rx: acq_result_rx,
                handle: acq_handle,
                started_at: now,
                last_touched: now,
                completed_at: None,
                state: InboundState::InProgress,
                skip_state,
            },
        );

        tracing::debug!(target: "inbound_ledger", seq, hash = %shamap_hash, "Acquire started");
        full_sync_debug!(
            "[full_debug][acq_request] spawned seq={} hash={} validated={} active_before={} skip_state={}",
            seq,
            debug_hash8(&hash),
            validated,
            self.entries
                .values()
                .filter(|e| matches!(e.state, InboundState::InProgress))
                .count(),
            skip_state
        );
        None
    }

    /// Poll all in-progress entries for results. Returns completed (hash, ledger, skip_state) pairs.
    fn poll_results(&mut self) -> Vec<(Uint256, ledger::Ledger, bool)> {
        let mut completed = Vec::new();
        let mut failed_hashes = Vec::new();

        for (hash, entry) in self.entries.iter_mut() {
            if !matches!(entry.state, InboundState::InProgress) {
                continue;
            }
            // Drain all pending results
            let mut last_result = None;
            loop {
                match entry.result_rx.try_recv() {
                    Ok(msg) => {
                        if matches!(&msg, AcqResult::Progress { .. }) {
                            entry.last_touched = Instant::now();
                        }
                        full_sync_debug!(
                            "[full_debug][acq_poll] recv seq={} hash={} msg={}",
                            entry.seq,
                            debug_hash8(hash),
                            match &msg {
                                AcqResult::Complete(_) => "complete",
                                AcqResult::Failed => "failed",
                                AcqResult::Progress { .. } => "progress",
                            }
                        );
                        last_result = Some(msg);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        full_sync_debug!(
                            "[full_debug][acq_poll] disconnected seq={} hash={}",
                            entry.seq,
                            debug_hash8(hash)
                        );
                        // Only treat as failure if still InProgress — completed
                        // entries have a disconnected channel because the worker
                        // exited after sending the result.
                        if last_result.is_none() && matches!(entry.state, InboundState::InProgress)
                        {
                            failed_hashes.push(*hash);
                        }
                        break;
                    }
                }
            }
            match last_result {
                Some(AcqResult::Complete(ledger)) => {
                    let skip = entry.skip_state;
                    completed.push((*hash, ledger, skip));
                }
                Some(AcqResult::Failed) => {
                    failed_hashes.push(*hash);
                }
                Some(AcqResult::Progress { .. }) => {}
                None => {}
            }
        }

        // Mark completed entries and notify for immediate storeLedger
        for (hash, ledger, _) in &completed {
            if let Some(entry) = self.entries.get_mut(hash) {
                entry.state = InboundState::Complete(ledger.clone());
                entry.last_touched = Instant::now();
                entry.completed_at = Some(Instant::now());
                // Immediately notify for storeLedger (matching rippled done() → storeLedger)
                full_sync_debug!(
                    "[full_debug][acq_poll] state_complete seq={} hash={} ledger_hash={} account_hash={} tx_hash={}",
                    entry.seq,
                    debug_hash8(hash),
                    ledger.header().hash,
                    ledger.header().account_hash,
                    ledger.header().tx_hash
                );
            }
        }

        // Mark failed entries
        for hash in &failed_hashes {
            self.recent_failures.insert(*hash, Instant::now());
            full_sync_debug!(
                "[full_debug][acq_poll] state_failed hash={} removed=true",
                debug_hash8(hash)
            );
            self.entries.remove(hash);
            self.registry.lock().expect("acq registry").remove(hash);
        }

        completed
    }

    #[allow(dead_code)]
    fn complete_ledger(&mut self, hash: &Uint256) -> Option<std::sync::Arc<ledger::Ledger>> {
        let entry = self.entries.get_mut(hash)?;
        entry.last_touched = Instant::now();
        full_sync_debug!(
            "[full_debug][acq_complete_lookup] seq={} hash={} state={}",
            entry.seq,
            debug_hash8(hash),
            entry.state.info_label()
        );
        match &entry.state {
            InboundState::Complete(ledger) => Some(std::sync::Arc::new(ledger.clone())),
            InboundState::InProgress | InboundState::Failed => None,
        }
    }

    /// Send peers to all in-progress acquisitions
    fn send_peers(&self, peers: &[Arc<dyn overlay::Peer>]) {
        for entry in self.entries.values() {
            if matches!(entry.state, InboundState::InProgress) {
                let _ = entry.tx.send(AcqMsg::Peers(peers.to_vec()));
            }
        }
    }

    fn sweep(&mut self) {
        let now = Instant::now();
        let before = self.entries.len();
        let mut to_remove = Vec::new();

        for (hash, entry) in &self.entries {
            match entry.state {
                InboundState::InProgress => {
                    // Remove stuck InProgress entries (reference 180s timeout)
                    if now.duration_since(entry.started_at) > INBOUND_STUCK_TIMEOUT
                        && now.duration_since(entry.last_touched) > INBOUND_SWEEP_INTERVAL
                    {
                        to_remove.push(*hash);
                    }
                }
                InboundState::Complete(_) | InboundState::Failed => {
                    // Remove completed/failed entries after 60s (reference sweep)
                    let sweep_since = entry.completed_at.unwrap_or(entry.last_touched);
                    if now.duration_since(sweep_since) > INBOUND_SWEEP_INTERVAL {
                        to_remove.push(*hash);
                    }
                }
            }
        }

        for hash in &to_remove {
            if let Some(entry) = self.entries.remove(hash) {
                full_sync_debug!(
                    "[full_debug][acq_sweep] remove seq={} hash={} state={} idle_ms={}",
                    entry.seq,
                    debug_hash8(hash),
                    entry.state.info_label(),
                    entry.last_touched.elapsed().as_millis()
                );
                let _ = entry.tx.send(AcqMsg::Stop);
                self.registry.lock().expect("acq registry").remove(hash);
            }
        }

        let swept = before - self.entries.len();
        if swept > 0 {
            tracing::debug!(target: "inbound_ledger", swept, before, "Swept entries");
        }

        // Expire old failures
        self.recent_failures
            .retain(|_, t| t.elapsed() < INBOUND_REACQUIRE_INTERVAL);
    }

    #[allow(dead_code)]
    fn stop(&mut self) {
        for (hash, entry) in self.entries.drain() {
            let _ = entry.tx.send(AcqMsg::Stop);
            self.registry.lock().expect("acq registry").remove(&hash);
        }
        self.recent_failures.clear();
    }

    /// Remove a specific entry (e.g., after accepting a ledger)
    fn remove(&mut self, hash: &Uint256) {
        if let Some(entry) = self.entries.remove(hash) {
            full_sync_debug!(
                "[full_debug][acq_remove] seq={} hash={} state={}",
                entry.seq,
                debug_hash8(hash),
                entry.state.info_label()
            );
            let _ = entry.tx.send(AcqMsg::Stop);
            self.registry.lock().expect("acq registry").remove(hash);
        }
    }

    /// Number of in-progress acquisitions
    fn active_count(&self) -> usize {
        self.entries
            .values()
            .filter(|e| matches!(e.state, InboundState::InProgress))
            .count()
    }

    fn remove_in_progress_below_seq(&mut self, min_seq: u32) -> usize {
        if min_seq <= 1 {
            return 0;
        }

        let stale_hashes = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                // Peers still serve content-addressed state nodes even after
                // the specific ledger seq falls below their min_seq.
                // Only retire completed/failed entries, not in-progress ones.
                !matches!(entry.state, InboundState::InProgress)
                    && entry.seq > 1
                    && entry.seq < min_seq
            })
            .map(|(hash, entry)| (*hash, entry.seq))
            .collect::<Vec<_>>();

        let count = stale_hashes.len();
        for (hash, seq) in stale_hashes {
            tracing::debug!(target: "inbound_ledger", seq, hash = %debug_hash8(&hash), min_seq, "Bootstrap retire unavailable");
            self.remove(&hash);
        }
        count
    }

    /// Log-visible summary shaped after reference InboundLedgers::getInfo.
    fn info_summary(&self) -> String {
        let mut entries = self
            .entries
            .iter()
            .map(|(hash, entry)| {
                let key = if entry.seq > 1 {
                    entry.seq.to_string()
                } else {
                    hash.to_string()
                };
                format!("{}:{}", key, entry.state.info_label())
            })
            .collect::<Vec<_>>();
        entries.sort();
        format!(
            "active={} complete={} failed={} entries=[{}]",
            self.active_count(),
            self.entries
                .values()
                .filter(|entry| matches!(entry.state, InboundState::Complete(_)))
                .count(),
            self.recent_failures.len(),
            entries.join(",")
        )
    }

    /// Total entries (including cached completions)
    #[allow(dead_code)]
    fn size(&self) -> usize {
        self.entries.len()
    }

    /// Check if we have a specific hash (in any state)
    fn contains(&self, hash: &Uint256) -> bool {
        self.entries.contains_key(hash)
    }

    fn is_in_progress(&self, hash: &Uint256) -> bool {
        self.entries
            .get(hash)
            .is_some_and(|entry| matches!(entry.state, InboundState::InProgress))
    }

    /// Get the seq for an entry
    #[allow(dead_code)]
    fn get_seq(&self, hash: &Uint256) -> Option<u32> {
        self.entries.get(hash).map(|e| e.seq)
    }
}

/// Acquisition processing thread — exact reference InboundLedger threading model.
///
///   Network thread → gotData() queues packet, dispatches runData once
///   Job thread     → runData() drains queue, processes, samples best peers, triggers each
///   Timer thread   → onTimer() every 3s, trigger(nullptr) broadcasts
///
/// This thread combines the job thread and timer thread roles.
/// The main catchup loop acts as the network thread, sending packets via channel.
fn run_acquisition_thread(
    rx: std::sync::mpsc::Receiver<AcqMsg>,
    result_tx: std::sync::mpsc::Sender<AcqResult>,
    hash: basics::sha_map_hash::SHAMapHash,
    seq: u32,
    ns: app::SHAMapStoreNodeStore,
    shared_tree_cache: std::sync::Arc<
        shamap::tree_node_cache::TreeNodeCache<basics::tagged_cache::MonotonicClock>,
    >,
    shared_full_below: std::sync::Arc<
        shamap::family::FullBelowCacheImpl<
            basics::tagged_cache::MonotonicClock,
            basics::hardened_hash::HardenedHashBuilder,
        >,
    >,
    shared_fetch_pack: Arc<ledger::FetchPackCache>,
    shared_write_tx: std::sync::mpsc::Sender<NodeStoreWriteMsg>,
    shared_pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
    run_data_limiter: Arc<RunDataLimiter>,
    shared_stored: Arc<basics::tagged_cache::KeyCache<Uint256>>,
    skip_state: bool,
    store_tx: std::sync::mpsc::Sender<Arc<ledger::Ledger>>,
) {
    use shamap::family::{NullMissingNodeReporter, SHAMapFamily};

    let tree_cache = shared_tree_cache;
    let full_below = shared_full_below;
    // marked complete for ledger N are still complete for ledger N+1
    // (they share 99%+ of state). Only clear on gap (handled in the
    // acceptance path when seq > validated + 1).
    let mut fetch_pack = SharedFetchPack::new(shared_fetch_pack);
    let mut inbound = ledger::InboundLedgerLocal::new(hash, seq);
    inbound.skip_state = skip_state;
    let peer_set = overlay::SimplePeerSet::new(std::iter::empty::<Arc<dyn overlay::Peer>>());
    let mut first_add_peers = true;
    let mut last_timer = Instant::now();
    let mut last_counter_log = Instant::now();
    let journal = CatchupJournal;
    let config = ledger::LedgerConfig::default();
    let counters = Arc::new(AcqCounters::default());
    let mut outbound_requests = 0u64;
    let mut last_request_at = Instant::now();
    let mut total_response_latency_ms = 0u64;
    let mut response_count = 0u64;
    let mut last_reported_response_count = 0u64;
    let mut last_debug_outbound_requests = 0u64;
    let mut last_debug_response_count = 0u64;
    let mut last_debug_write_count = 0u64;
    let mut acq_peer_stats: HashMap<u64, AcqPeerPacketStats> = HashMap::new();
    const PEER_COUNT_START: usize = 5;
    const PEER_COUNT_ADD: usize = 3;

    // and dispatches runData as a job. The network thread keeps receiving
    // while runData processes. By the time runData finishes, responses from
    // ALL peers are queued. This creates pipeline depth = peer count.
    //
    // Rust equivalent: spawn a receiver thread that continuously drains the
    // mpsc channel into a shared queue. The main thread processes the queue,
    // triggers peers, and immediately checks the queue again. The receiver
    // keeps filling the queue while the processor works.
    let shared_queue: Arc<Mutex<Vec<AcqMsg>>> = Arc::new(Mutex::new(Vec::new()));
    let queue_condvar = Arc::new(std::sync::Condvar::new());

    // Receiver thread: drains mpsc channel into shared queue (reference gotData role)
    let recv_queue = Arc::clone(&shared_queue);
    let recv_condvar = Arc::clone(&queue_condvar);
    let _receiver_handle = thread::Builder::new()
        .name("xrpld-acq-recv".to_owned())
        .spawn(move || {
            loop {
                match rx.recv() {
                    Ok(msg) => {
                        let is_stop = matches!(msg, AcqMsg::Stop);
                        {
                            let mut queue = recv_queue.lock().expect("acq queue");
                            queue.push(msg);
                            // Drain any additional messages without blocking
                            while let Ok(extra) = rx.try_recv() {
                                let extra_stop = matches!(extra, AcqMsg::Stop);
                                queue.push(extra);
                                if extra_stop {
                                    break;
                                }
                            }
                        }
                        recv_condvar.notify_one();
                        if is_stop {
                            break;
                        }
                    }
                    Err(_) => break, // channel disconnected
                }
            }
        })
        .expect("acq receiver thread");

    // Track whether we just processed data to keep the pipeline hot.
    let mut just_processed = false;
    let mut loop_iterations = 0u64;
    let mut data_iterations = 0u64;
    let mut empty_wakeups = 0u64;
    let mut stopped = false;

    // Dedicated NuDB writer thread — all writes go through this channel
    // so acquisition threads never block on store_mutex.
    let mut store = NodeStoreWriter {
        node_store: ns.clone(),
        counters: counters.clone(),
        write_tx: shared_write_tx,
        write_count: std::cell::Cell::new(0),
        pending_writes: Arc::clone(&shared_pending_writes),
        shared_stored,
    };

    // NOTE: Initial tryDB (check_local) removed — it produces false completions
    // with seq=0 for hashes that share state roots with genesis. The normal
    // InboundLedger flow correctly downloads headers and builds proper ledgers.

    // Processor loop (reference runData role): processes queue, triggers peers
    loop {
        // Wait for data or timer — like reference job queue waiting for dispatch.
        // After processing data, use a very short timeout (1ms) to immediately
        // pick up responses that arrived while we were processing. This matches
        let msgs = {
            loop_iterations += 1;
            let mut queue = shared_queue.lock().expect("acq queue");
            if queue.is_empty() {
                let timeout = if last_timer.elapsed() >= Duration::from_secs(1) {
                    Duration::from_millis(0)
                } else if just_processed {
                    // Pipeline hot: check again quickly for responses
                    Duration::from_millis(1)
                } else {
                    Duration::from_secs(1)
                        .saturating_sub(last_timer.elapsed())
                        .min(Duration::from_millis(50))
                };
                if !timeout.is_zero() {
                    let (guard, _) = queue_condvar
                        .wait_timeout(queue, timeout)
                        .expect("acq condvar");
                    queue = guard;
                }
            }
            std::mem::take(&mut *queue)
        };

        let mut got_stop = false;
        let mut fetch_pack_ready = false;
        let mut peers_updated = false;
        let msg_count = msgs.len();
        if msg_count == 0 {
            empty_wakeups += 1;
        }
        for msg in msgs {
            match msg {
                AcqMsg::LedgerData { peer_id, packet } => {
                    counters.inc_ledger_packet();
                    let latency = last_request_at.elapsed().as_millis() as u64;
                    total_response_latency_ms += latency;
                    response_count += 1;
                    if acq_packet_debug_enabled() {
                        let shape = ledger::InboundLedgerPacketShape::classify(&packet);
                        let last_request_nodes = acq_peer_stats
                            .get(&peer_id)
                            .map(|stats| stats.last_request_nodes)
                            .unwrap_or(0);
                        let fat_yield_milli = if last_request_nodes > 0 {
                            shape.nodes.saturating_mul(1000) / last_request_nodes
                        } else {
                            0
                        };
                        tracing::debug!(target: "inbound_ledger",
                            seq, peer_id, packet_type = ?packet.packet_type,
                            nodes = shape.nodes, inner = shape.inner_nodes,
                            leaf = shape.leaf_nodes, malformed = shape.malformed_nodes,
                            empty = shape.empty_nodes, last_request_nodes,
                            fat_yield_milli, latency_ms = latency, "Packet received"
                        );
                    }
                    let _ = inbound.got_data(Some(peer_id), packet);
                }
                AcqMsg::FetchPackReady => {
                    fetch_pack_ready = true;
                }
                AcqMsg::Peers(p) => {
                    peer_set.refresh_peers(p.iter().cloned());
                    peers_updated = true;
                }
                AcqMsg::Stop => {
                    got_stop = true;
                }
            }
        }
        if got_stop {
            stopped = true;
            break;
        }

        // Skip run_data processing if no data queued, but always check
        // addPeers and timer. The old code skipped everything when no data
        // was queued, which prevented addPeers from triggering requests.
        let has_queued_data = inbound.received_data_len() > 0;
        let timer_due = last_timer.elapsed() >= Duration::from_secs(1);
        if !should_process_acquisition_tick(
            has_queued_data,
            timer_due,
            fetch_pack_ready,
            first_add_peers,
            peers_updated,
        ) {
            // Check completion before spinning — trigger may have set
            // have_state/have_tx on a previous iteration.
            if !inbound.is_done()
                && inbound.planner_state().have_header
                && inbound.planner_state().have_state
                && inbound.planner_state().have_transactions
            {
                inbound.set_complete();
            }
            if inbound.is_done() {
                break;
            }
            just_processed = false;
            continue;
        }

        // Create family once per processing batch
        let family = SHAMapFamily::new(
            tree_cache.clone(),
            &*full_below,
            NodeStoreFetcher {
                node_store: ns.clone(),
                pending_writes: Arc::clone(&shared_pending_writes),
            },
            NullMissingNodeReporter,
        );

        if fetch_pack_ready {
            // which immediately calls InboundLedger::checkLocal on every active
            // acquisition after the shared fetch-pack cache is populated.
            let completed_from_fetch_pack = inbound.check_local_with_family_and_config(
                &journal,
                &config,
                &mut store,
                &mut fetch_pack,
                &family,
            );
            if completed_from_fetch_pack {
                full_sync_debug!(
                    "[full_debug][acq_worker] fetch_pack_completed seq={} hash={}",
                    seq,
                    debug_hash8(hash.as_uint256())
                );
            }
        }

        // Later peer expansion happens only from onTimer(false), not from
        // normal runData processing.
        if first_add_peers {
            let acq_hash = *hash.as_uint256();
            let mut newly_added = Vec::new();
            peer_set.add_peers(
                PEER_COUNT_START,
                &mut |peer| peer.has_ledger(acq_hash, seq),
                &mut |peer| newly_added.push(Arc::clone(peer)),
            );
            // Fallback: if no peers claim to have the ledger, send blind requests
            if newly_added.is_empty() {
                peer_set.add_peers(PEER_COUNT_START, &mut |_peer| true, &mut |peer| {
                    newly_added.push(Arc::clone(peer))
                });
            }
            for peer in &newly_added {
                let peer_ref = peer.clone();
                let mut send_fn = |msg: overlay::ProtocolMessage| {
                    outbound_requests += 1;
                    if acq_packet_debug_enabled() {
                        let (itype, requested, query_depth) = get_ledger_request_shape(&msg);
                        acq_peer_stats
                            .entry(peer_ref.id() as u64)
                            .or_default()
                            .record_request(requested);
                        tracing::debug!(target: "inbound_ledger",
                            seq, peer = peer_ref.id(), itype, requested,
                            query_depth = query_depth.map(|v| v.to_string()).unwrap_or_else(|| "none".to_owned()),
                            outbound_requests, reason = "added", "Request send"
                        );
                    }
                    let wire = overlay::Message::new(msg, None);
                    peer_ref.send(wire);
                };
                inbound.trigger_with_family(
                    ledger::InboundLedgerRequestTrigger::Added,
                    &journal,
                    &config,
                    &mut store,
                    &mut fetch_pack,
                    &family,
                    &mut send_fn,
                );
                last_request_at = Instant::now();
            }
            if !newly_added.is_empty() {
                first_add_peers = false;
                last_timer = Instant::now();
            }
        }

        // --- reference runData: process all queued packets ---
        if inbound.received_data_len() > 0 {
            // The receiver thread continuously fills the shared queue while
            // we process. By the time we finish processing batch N, responses
            // from all peers for batch N's requests are already queued.
            // This matches reference where runData processes on a job thread while
            // the network thread keeps receiving via gotData.

            let run_start = Instant::now();

            let write_count_before = store.write_count.get();
            // Limit concurrent run_data to match reference 6-thread job queue
            run_data_limiter.acquire();
            let run_result = inbound.run_data_with_family_and_config_and_refill(
                &journal,
                &config,
                &mut store,
                &mut fetch_pack,
                &family,
                &mut || {
                    let mut refill = Vec::new();
                    let mut queue = shared_queue.lock().expect("acq queue");
                    if queue.is_empty() {
                        return refill;
                    }

                    let mut retained = Vec::new();
                    for msg in std::mem::take(&mut *queue) {
                        match msg {
                            AcqMsg::LedgerData { peer_id, packet } => {
                                counters.inc_ledger_packet();
                                let latency = last_request_at.elapsed().as_millis() as u64;
                                total_response_latency_ms += latency;
                                response_count += 1;
                                if acq_packet_debug_enabled() {
                                    let shape = ledger::InboundLedgerPacketShape::classify(&packet);
                                    let last_request_nodes = acq_peer_stats
                                        .get(&peer_id)
                                        .map(|stats| stats.last_request_nodes)
                                        .unwrap_or(0);
                                    let fat_yield_milli = if last_request_nodes > 0 {
                                        shape.nodes.saturating_mul(1000) / last_request_nodes
                                    } else {
                                        0
                                    };
                                    tracing::debug!(target: "inbound_ledger",
                                        seq, peer_id, packet_type = ?packet.packet_type,
                                        nodes = shape.nodes, inner = shape.inner_nodes,
                                        leaf = shape.leaf_nodes, malformed = shape.malformed_nodes,
                                        empty = shape.empty_nodes, last_request_nodes,
                                        fat_yield_milli, latency_ms = latency,
                                        source = "refill", "Packet received"
                                    );
                                }
                                refill.push(ledger::InboundLedgerReceivedPacket::new(
                                    Some(peer_id),
                                    packet,
                                ));
                            }
                            other => retained.push(other),
                        }
                    }
                    *queue = retained;
                    refill
                },
            );
            run_data_limiter.release();
            let run_elapsed = run_start.elapsed();

            counters.inc_run_data_batch(run_result.processed_packets);
            if acq_packet_debug_enabled() {
                for stats in &run_result.packet_stats {
                    if let Some(peer_id) = stats.peer_id {
                        acq_peer_stats
                            .entry(peer_id)
                            .or_default()
                            .record_packet(stats);
                    }
                }
            }
            just_processed = run_result.processed_packets > 0;
            if just_processed {
                data_iterations += 1;
            }

            if run_elapsed.as_millis() > 100 {
                let run_ms = run_elapsed.as_millis();
                let unique_sends = store.write_count.get() - write_count_before;

                tracing::debug!(target: "inbound_ledger", seq, run_ms, unique_sends, pkts = run_result.processed_packets, "Run data profile");
            }

            // productive peers, then calls trigger(peer, Reply) for EACH
            // sampled peer. Each trigger call adds requested nodes to
            // recent_nodes, so subsequent peers get different node sets.
            // This distributes requests across peers instead of broadcasting.
            // Note: trigger_with_family calls finish_if_done internally, which
            // sets signaled=true when complete. We must not skip this call.
            if peer_set.peer_count() > 0 {
                let trigger_start = Instant::now();
                let trigger_count = run_result.triggered_peer_ids.len();
                // Trigger sampled productive peers (compatibility)
                for peer_id in &run_result.triggered_peer_ids {
                    let Some(peer) = peer_set.find_peer(*peer_id as u32) else {
                        continue;
                    };
                    let peer_ref = peer.clone();
                    let trigger_reason = if peer_ref.is_high_latency() {
                        ledger::InboundLedgerRequestTrigger::ReplyHighLatency
                    } else {
                        ledger::InboundLedgerRequestTrigger::Reply
                    };
                    let mut send_fn = |msg: overlay::ProtocolMessage| {
                        outbound_requests += 1;
                        if acq_packet_debug_enabled() {
                            let (itype, requested, query_depth) = get_ledger_request_shape(&msg);
                            acq_peer_stats
                                .entry(*peer_id)
                                .or_default()
                                .record_request(requested);
                            tracing::debug!(target: "inbound_ledger",
                                seq, peer_id, itype, requested,
                                query_depth = query_depth.map(|v| v.to_string()).unwrap_or_else(|| "none".to_owned()),
                                outbound_requests, reason = "reply", "Request send"
                            );
                        }
                        peer_set.send_request(&msg, Some(&peer_ref));
                    };
                    inbound.trigger_with_family(
                        trigger_reason,
                        &journal,
                        &config,
                        &mut store,
                        &mut fetch_pack,
                        &family,
                        &mut send_fn,
                    );
                }

                let trigger_elapsed = trigger_start.elapsed();
                tracing::debug!(target: "inbound_ledger", seq, trigger_count, tracked_peers = peer_set.peer_count(), outbound_requests, trigger_ms = trigger_elapsed.as_millis(), "Reply triggered");

                // Cold-start parallel fan-out: after the reply trigger (which
                // targets the responding peer), trigger additional peers with
                // different missing nodes. `recent_nodes` ensures each peer gets
                // a unique partition — nodes already requested are filtered out,
                // so each subsequent trigger call discovers fresh work.
                if !inbound.planner_state().have_state {
                    // Refresh tracked peers from overlay so fan-out can use all connected peers
                    peer_set.add_peers(6, &mut |_peer| true, &mut |_peer| {});
                    let all_peers = peer_set.get_peers();
                    let triggered: std::collections::HashSet<u32> = run_result
                        .triggered_peer_ids
                        .iter()
                        .map(|id| *id as u32)
                        .collect();
                    let extra_peers: Vec<_> = all_peers
                        .iter()
                        .filter(|p| !triggered.contains(&p.id()))
                        .take(5)
                        .cloned()
                        .collect();
                    for peer in &extra_peers {
                        let peer_ref = peer.clone();
                        let mut fan_send = |msg: overlay::ProtocolMessage| {
                            outbound_requests += 1;
                            peer_set.send_request(&msg, Some(&peer_ref));
                        };
                        inbound.trigger_with_family(
                            ledger::InboundLedgerRequestTrigger::Reply,
                            &journal,
                            &config,
                            &mut store,
                            &mut fetch_pack,
                            &family,
                            &mut fan_send,
                        );
                    }
                }

                last_request_at = Instant::now();
            }
        }

        // processing data and sets complete_=true + calls done() inline.
        // Our trigger_with_family does this via getMissingNodes, but trigger
        // may not be called if no peers were sampled. Check completion directly.
        if !inbound.is_done()
            && inbound.planner_state().have_header
            && inbound.planner_state().have_state
            && inbound.planner_state().have_transactions
            && !inbound.is_complete()
        {
            inbound.set_complete();
        }

        // finish_if_done (which sets signaled) when both maps report no
        // missing nodes. Once signaled, is_done() becomes true.
        if inbound.is_done() {
            break;
        }

        // --- reference onTimer: every 3 seconds ---
        if last_timer.elapsed() >= Duration::from_secs(3) {
            last_timer = Instant::now();
            let was_progress = inbound.progress();
            if was_progress {
                // recentNodes_ and returns. It does not add peers and does
                // not send timeout-triggered requests.
                inbound.clear_progress();
                inbound.clear_recent_nodes();
            } else {
                // - Generic/consensus: trigger existing peers with Timeout, then addPeers()
                //   triggers each new peer with Added.
                // - History: addPeers() first, then trigger Timeout, because the fetch-pack
                //   path should not trigger newly added peers too early.
                // Each Added trigger updates recent_nodes, so newly added peers receive
                // different node IDs instead of one duplicate broadcast set.
                let peer_limit = if peer_set.peer_count() == 0 {
                    PEER_COUNT_START
                } else {
                    PEER_COUNT_ADD
                };
                let acq_hash = *hash.as_uint256();

                if inbound.reason() == ledger::InboundLedgerReason::History {
                    peer_set.add_peers(
                        peer_limit,
                        &mut |peer| peer.has_ledger(acq_hash, seq),
                        &mut |_peer| {},
                    );
                    if first_add_peers && peer_set.peer_count() > 0 {
                        first_add_peers = false;
                    }
                    let mut send_fn = |msg: overlay::ProtocolMessage| {
                        outbound_requests += 1;
                        if acq_packet_debug_enabled() {
                            let (itype, requested, query_depth) = get_ledger_request_shape(&msg);
                            tracing::debug!(target: "inbound_ledger",
                                seq, peer = "all", itype, requested,
                                query_depth = query_depth.map(|v| v.to_string()).unwrap_or_else(|| "none".to_owned()),
                                outbound_requests, reason = "timeout", "Request send"
                            );
                        }
                        peer_set.send_request(&msg, None);
                    };
                    let failed = inbound.on_timer_with_family(
                        &journal,
                        &config,
                        &mut store,
                        &mut fetch_pack,
                        &family,
                        &mut send_fn,
                    );
                    if failed {
                        tracing::warn!(target: "inbound_ledger", seq, "Timer failure — retiring");
                        break;
                    }
                } else {
                    let mut send_fn = |msg: overlay::ProtocolMessage| {
                        outbound_requests += 1;
                        if acq_packet_debug_enabled() {
                            let (itype, requested, query_depth) = get_ledger_request_shape(&msg);
                            tracing::debug!(target: "inbound_ledger",
                                seq, peer = "all", itype, requested,
                                query_depth = query_depth.map(|v| v.to_string()).unwrap_or_else(|| "none".to_owned()),
                                outbound_requests, reason = "timeout", "Request send"
                            );
                        }
                        peer_set.send_request(&msg, None);
                    };
                    let failed = inbound.on_timer_with_family(
                        &journal,
                        &config,
                        &mut store,
                        &mut fetch_pack,
                        &family,
                        &mut send_fn,
                    );
                    if failed {
                        tracing::warn!(target: "inbound_ledger", seq, "Timer failure — retiring");
                        break;
                    }

                    let mut newly_added = Vec::new();
                    peer_set.add_peers(
                        peer_limit,
                        &mut |peer| peer.has_ledger(acq_hash, seq),
                        &mut |peer| newly_added.push(Arc::clone(peer)),
                    );
                    for peer in &newly_added {
                        let peer_ref = peer.clone();
                        let mut send_fn = |msg: overlay::ProtocolMessage| {
                            outbound_requests += 1;
                            if acq_packet_debug_enabled() {
                                let (itype, requested, query_depth) =
                                    get_ledger_request_shape(&msg);
                                acq_peer_stats
                                    .entry(peer_ref.id() as u64)
                                    .or_default()
                                    .record_request(requested);
                                tracing::debug!(target: "inbound_ledger",
                                    seq, peer = peer_ref.id(), itype, requested,
                                    query_depth = query_depth.map(|v| v.to_string()).unwrap_or_else(|| "none".to_owned()),
                                    outbound_requests, reason = "added", "Request send"
                                );
                            }
                            peer_ref.send(overlay::Message::new(msg, None));
                        };
                        inbound.trigger_with_family(
                            ledger::InboundLedgerRequestTrigger::Added,
                            &journal,
                            &config,
                            &mut store,
                            &mut fetch_pack,
                            &family,
                            &mut send_fn,
                        );
                        last_request_at = Instant::now();
                    }
                    if first_add_peers && !newly_added.is_empty() {
                        first_add_peers = false;
                    }
                }
            }
        }

        if last_counter_log.elapsed() >= Duration::from_secs(5) {
            last_counter_log = Instant::now();
            counters.log_status(seq, inbound.stats(), inbound.is_complete());
            let write_count = store.write_count.get();
            let outbound_delta = outbound_requests.saturating_sub(last_debug_outbound_requests);
            let response_delta = response_count.saturating_sub(last_debug_response_count);
            let write_delta = write_count.saturating_sub(last_debug_write_count);
            last_debug_outbound_requests = outbound_requests;
            last_debug_response_count = response_count;
            last_debug_write_count = write_count;
            if inbound.progress() || response_count > last_reported_response_count {
                let _ = result_tx.send(AcqResult::Progress {
                    good_nodes: inbound.stats().get_good().max(0) as usize,
                });
                last_reported_response_count = response_count;
            }
            let avg_latency = if response_count > 0 {
                total_response_latency_ms / response_count
            } else {
                0
            };
            tracing::debug!(target: "inbound_ledger",
                seq, outbound_requests, peers = peer_set.peer_count(),
                outbound_delta, response_delta, write_delta,
                progress = inbound.progress(),
                have_header = inbound.planner_state().have_header,
                have_state = inbound.planner_state().have_state,
                have_tx = inbound.planner_state().have_transactions,
                response_count, avg_latency, loop_iterations, data_iterations, empty_wakeups,
                "Acquisition flow"
            );
            if acq_packet_debug_enabled() {
                for (peer_id, stats) in acq_peer_stats.iter().take(16) {
                    let useful_ratio_milli = if stats.nodes > 0 {
                        stats.useful.saturating_mul(1000) / stats.nodes
                    } else {
                        0
                    };
                    let duplicate_ratio_milli = if stats.nodes > 0 {
                        stats.duplicate.saturating_mul(1000) / stats.nodes
                    } else {
                        0
                    };
                    let avg_elapsed_ms = if stats.packets > 0 {
                        stats.elapsed_ms / stats.packets
                    } else {
                        0
                    };
                    tracing::debug!(target: "inbound_ledger",
                        seq, peer_id,
                        packets = stats.packets, nodes = stats.nodes,
                        inner = stats.inner_nodes, leaf = stats.leaf_nodes,
                        malformed = stats.malformed_nodes, useful = stats.useful,
                        invalid = stats.invalid, duplicate = stats.duplicate,
                        useful_ratio_milli, duplicate_ratio_milli, avg_elapsed_ms,
                        last_request_nodes = stats.last_request_nodes,
                        "Peer yield"
                    );
                }
            }
        }

        // Check completion — reference calls done() inline in trigger() and never
        // re-verifies with a second walk. Once complete=true, accept immediately.
        if inbound.is_complete() {
            if inbound.is_failed() {
                break;
            }
            let Some(ledger) = inbound.ledger().cloned() else {
                tracing::warn!(target: "inbound_ledger", seq, "Accepted completion missing ledger after owner acceptance");
                break;
            };
            if !flush_nodestore_writes(&store.write_tx) {
                tracing::warn!(target: "inbound_ledger", seq, "Failed to flush nodestore writes before completion");
                break;
            }
            tracing::info!(target: "inbound_ledger", seq, "LEDGER ACQUIRED");
            counters.log_status(seq, inbound.stats(), true);
            full_sync_debug!(
                "[full_debug][acq_worker] send_complete seq={} hash={} ledger_hash={} account_hash={} tx_hash={} fees_base={} fees_reserve={} fees_inc={} state_full={} tx_full={}",
                seq,
                debug_hash8(hash.as_uint256()),
                ledger.header().hash,
                ledger.header().account_hash,
                ledger.header().tx_hash,
                ledger.fees().base,
                ledger.fees().reserve,
                ledger.fees().increment,
                ledger.state_map().is_full(),
                ledger.tx_map().is_full()
            );
            let _ = store_tx.send(Arc::new(ledger.clone()));
            if result_tx.send(AcqResult::Complete(ledger)).is_err() {
                tracing::warn!(target: "inbound_ledger", seq, "Completion receiver dropped before catchup consumed result");
                full_sync_debug!(
                    "[full_debug][acq_worker] send_complete_failed seq={} hash={} reason=receiver_dropped",
                    seq,
                    debug_hash8(hash.as_uint256())
                );
            }
            return;
        }
    }
    if inbound.is_complete() && !inbound.is_failed() && !stopped {
        // The is_done() break fires before the inline completion handler.
        // Build the ledger and send Complete here so poll_results receives it.
        if let Some(ledger) = inbound.ledger().cloned() {
            let _ = flush_nodestore_writes(&store.write_tx);
            tracing::info!(target: "inbound_ledger", seq, "LEDGER ACQUIRED");
            counters.log_status(seq, inbound.stats(), true);
            let _ = store_tx.send(Arc::new(ledger.clone()));
            if result_tx.send(AcqResult::Complete(ledger)).is_err() {
                tracing::warn!(target: "inbound_ledger", seq, "Completion receiver dropped before catchup consumed result");
            }
        }
    } else if inbound.is_failed() && !stopped {
        full_sync_debug!(
            "[full_debug][acq_worker] send_failed seq={} hash={} have_header={} have_state={} have_tx={} stats_good={} stats_bad={} stats_dup={}",
            seq,
            debug_hash8(hash.as_uint256()),
            inbound.planner_state().have_header,
            inbound.planner_state().have_state,
            inbound.planner_state().have_transactions,
            inbound.stats().get_good(),
            inbound.stats().get_bad(),
            inbound.stats().get_duplicate(),
        );
        let _ = result_tx.send(AcqResult::Failed);
    }
}
/// Serve a GetLedger request from a peer by resolving an immutable ledger
/// through the app-owned loaded-ledger runtime, then replying from that ledger.
/// Matches the the reference implementation `PeerImp::getLedger` + `processLedgerRequest` split.
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
        self.catch_up_state.stop.store(false, Ordering::Release);
        let mut app = self.app.clone();
        let state = Arc::clone(&self.catch_up_state);
        let node_store_usage_path = self.node_store_usage_path.clone();
        let peerfinder_bootcache_path = self.peerfinder_bootcache_path.clone();
        let ledger_fetch_limit_override = self.ledger_fetch_limit_override;
        let rt_handle_persist = tokio::runtime::Handle::current();
        let handle = thread::Builder::new()
            .name("xrpld-ledger-catchup".to_owned())
            .spawn(move || {

                // InboundLedgers manager — compatibility: single entry point for
                // all ledger acquisition with dedup, caching, and sweep.
                let acq_registry: AcqRegistry = Arc::new(Mutex::new(HashMap::new()));
                // InboundLedgers manager replaces persistent_acqs
                let catchup_profile =
                    CatchupResourceProfile::for_node_size(app.status_rpc_node_size().as_deref())
                        .with_ledger_fetch_limit_override(ledger_fetch_limit_override);
                // Shared tree-node cache and full-below cache across all
                // acquisition threads, matching reference where all InboundLedgers
                // share the same NodeFamily's caches. This prevents duplicate
                // fetches when consecutive ledgers share most state-tree nodes.
                let shared_tree_cache = {
                    use basics::tagged_cache::MonotonicClock;
                    use shamap::tree_node_cache::TreeNodeCache;
                    Arc::new(TreeNodeCache::new(
                        "acq",
                        catchup_profile.acq_tree_cache_size,
                        time::Duration::seconds(catchup_profile.acq_tree_cache_age_seconds),
                        MonotonicClock::default(),
                    ))
                };
                let shared_full_below = {
                    use basics::hardened_hash::HardenedHashBuilder;
                    use basics::tagged_cache::MonotonicClock;
                    use shamap::family::FullBelowCacheImpl;
                    Arc::new(FullBelowCacheImpl::new(
                        1,
                        MonotonicClock::default(),
                        HardenedHashBuilder::default(),
                        catchup_profile.acq_full_below_size,
                    ))
                };
                // Shared fetch-pack cache matching reference LedgerMaster::fetch_packs_.
                // All acquisitions read from this cache via AccountStateSF::getNode.
                // Fetch-pack replies and stale data populate it.
                let shared_fetch_pack = {
                    use basics::tagged_cache::MonotonicClock;
                    Arc::new(ledger::FetchPackCache::new(
                        catchup_profile.acq_fetch_pack_size,
                        time::Duration::seconds(45),
                        MonotonicClock::default(),
                    ))
                };
                let shared_pending_writes =
                    Arc::new(Mutex::new(HashMap::<Uint256, PendingNodeStoreObject>::new()));

                if let Some(ns) = app.node_store().as_ref() {
                    let family = shamap::family::SHAMapFamily::new(
                        Arc::clone(&shared_tree_cache),
                        Arc::clone(&shared_full_below),
                        NodeStoreFetcher {
                            node_store: ns.clone(),
                            pending_writes: Arc::clone(&shared_pending_writes),
                        },
                        shamap::family::NullMissingNodeReporter,
                    );
                    let node_family: Arc<dyn app::node_family::node_family::NodeFamilyRuntime> =
                        Arc::new(app::node_family::node_family::NodeFamily::new(family));
                    let _ = app.attach_node_family(node_family);
                }

                // After the first ledger completes, it may pursue a few more
                // concurrently but stays conservative to avoid OOM on smaller
                // machines. Match that behavior and always start from the
                // network-ledger path rather than auto-resuming a local
                // validated ledger snapshot.

                let catchup_start = Instant::now();
                let _milestone_first_header: Option<Duration> = None;
                let mut milestone_first_ledger: Option<Duration> = None;
                let _milestone_first_fetch_pack: Option<Duration> = None;

                // Single shared NuDB writer thread for ALL acquisitions.
                // Eliminates store_mutex contention from acquisition threads.
                let (shared_write_tx, _shared_writer_handle) = if let Some(ns) = app.node_store() {
                    let (tx, handle) =
                        spawn_nodestore_writer(ns.clone(), Arc::clone(&shared_pending_writes));
                    (Some(tx), Some(handle))
                } else {
                    (None, None)
                };

                // concurrency limit to reduce cache mutex contention.
                let run_data_limiter =
                    Arc::new(RunDataLimiter::new(catchup_profile.run_data_concurrency));
                let shared_stored = Arc::new(basics::tagged_cache::KeyCache::new(
                    "acq-write-dedup",
                    catchup_profile.write_dedup_size,
                    time::Duration::seconds(30),
                    basics::tagged_cache::MonotonicClock::default(),
                ));

                // Wire InboundLedgers to shared resources.
                // new acquisitions through a global active-ledger cap.
                let (completed_ledgers_tx, completed_ledgers_rx) =
                    std::sync::mpsc::channel::<Arc<ledger::Ledger>>();
                let mut inbound_ledgers = InboundLedgers::new(
                    Arc::clone(&acq_registry),
                    Arc::clone(&shared_tree_cache),
                    Arc::clone(&shared_full_below),
                    Arc::clone(&shared_fetch_pack),
                    Arc::clone(&run_data_limiter),
                    Arc::clone(&shared_stored),
                    completed_ledgers_tx,
                );
                // Share the receiver with the ApplicationRoot so the bootstrap
                // thread can poll completed ledgers every 50ms (matching
                // rippled's done() → storeLedger() without polling delay).
                app.set_completed_ledgers_rx(completed_ledgers_rx);
                if let Some(ns) = app.node_store().as_ref() {
                    inbound_ledgers.set_node_store(ns.clone());
                }
                if let Some(ref tx) = shared_write_tx {
                    inbound_ledgers.set_write_tx(tx.clone());
                    inbound_ledgers.set_pending_writes(Arc::clone(&shared_pending_writes));
                if let Some(ort) = app.overlay_runtime() {
                    inbound_ledgers.set_overlay_rt(ort);
                }
                }

                let mut last_diag_at = Instant::now()
                    .checked_sub(Duration::from_secs(15))
                    .unwrap_or_else(Instant::now);
                let mut last_endpoints_at = Instant::now()
                    .checked_sub(PEERFINDER_SECONDS_PER_MESSAGE)
                    .unwrap_or_else(Instant::now);
                let mut last_auto_connect_at = Instant::now()
                    .checked_sub(PEERFINDER_SECONDS_PER_CONNECT)
                    .unwrap_or_else(Instant::now);
                let mut last_bootcache_save_at = Instant::now();
                let mut last_cache_sweep_at = Instant::now();
                let mut last_inbound_sweep_at = Instant::now();
                // reached quorum. The 1-second checkAccept timer re-checks this
                // in case the ledger arrived after the validation.
                let mut last_validated_target: Option<(Uint256, u32)> = None;
                // from SQLite (rdb), reconstruct Ledger from NuDB state root.
                // No peer acquisition needed — we already have the data locally.
                {
                tracing::debug!(target: "bootstrap", ledger_db = app.ledger_db().is_some(), "rdb: starting bootstrap");
                    let loaded = app.ledger_db()
                        .and_then(|db| db.get_newest_ledger_info().ok().flatten())
                        .and_then(|info| {
                            tracing::debug!(target: "bootstrap", seq = info.ledger_seq, hash = &info.ledger_hash[..8], "rdb: found ledger");
                            info.to_header()
                        })
                        .and_then(|header| {
                            let ns = app.node_store().clone()?;
                            tracing::debug!(target: "bootstrap", seq = header.seq, "rdb: node_store present, loading");
                            let family = shamap::family::SHAMapFamily::new(
                                Arc::clone(&shared_tree_cache),
                                &*shared_full_below,
                                NodeStoreFetcher {
                                    node_store: ns.clone(),
                                    pending_writes: Arc::clone(&shared_pending_writes),
                                },
                                shamap::family::NullMissingNodeReporter,
                            );
                            let journal = ledger::NullLedgerJournal;
                            let config = if let Some(lm) = app.ledger_master_runtime() {
                                let _ = lm; // just to confirm it exists
                                ledger::LedgerConfig::default()
                            } else {
                                return None;
                            };
                            match ledger::load_ledger_helper(header, false, &journal, &config, &family) {
                                Ok(Some(ledger)) => {
                                    tracing::info!(target: "bootstrap", seq = header.seq, "rdb: loaded from NuDB ✓");
                                    Some(std::sync::Arc::new(ledger))
                                }
                                Ok(None) => {
                                    tracing::debug!(target: "bootstrap", seq = header.seq, "rdb: NuDB miss, will acquire from peers");
                                    None
                                }
                                Err(e) => {
                                    tracing::warn!(target: "bootstrap", seq = header.seq, ?e, "rdb: load error");
                                    None
                                }
                            }
                        });

                    if let Some(ledger) = loaded {
                        let seq = ledger.header().seq;
                        let hash = *ledger.header().hash.as_uint256();
                        if let Some(lm) = app.ledger_master_runtime() {
                            let ledger_master = lm.ledger_master();
                            let persistence = ledger::LedgerPersistence::new(std::sync::Arc::new(
                                app.build_ledger_persistence_runtime(),
                            ));
                            if let Err(e) = ledger_master.set_full_ledger(
                                &persistence,
                                std::sync::Arc::clone(&ledger),
                                true,
                                true,
                                None,
                                None,
                            ) {
                                tracing::warn!(target: "bootstrap", seq, ?e, "rdb: set_full_ledger failed");
                            } else {
                                let _ = app.on_validated_ledger(std::sync::Arc::clone(&ledger));
                                tracing::info!(target: "bootstrap", seq, "rdb: accepted as validated ✓ (from local NuDB)");
                                last_validated_target = Some((hash, seq));
                                // No peers yet at startup — advance_published_ledgers will run
                                // on the first loop iteration when peers are available.
                                advance_published_ledgers_after_validation(
                                    &app,
                                    &[],
                                    &mut inbound_ledgers,
                                    ledger,
                                );
                            }
                        }
                    } else {
                        // Fallback: load from last_validated.txt for nodes that don't have
                        // ledger_headers.db yet (first run after upgrade).
                        if let Some((hash, seq)) = std::fs::read_to_string("/mnt/xrpl-data/testnet/last_validated.txt")
                            .ok()
                            .and_then(|s| {
                                let parts: Vec<&str> = s.trim().split(' ').collect();
                                if parts.len() == 2 {
                                    let hex = parts[0];
                                    if hex.len() != 64 { return None; }
                                    let mut arr = [0u8; 32];
                                    for i in 0..32 {
                                        arr[i] = u8::from_str_radix(&hex[i*2..i*2+2], 16).ok()?;
                                    }
                                    let seq: u32 = parts[1].parse().ok()?;
                                    tracing::debug!(target: "bootstrap", seq, "Fallback: last_validated.txt loaded");
                                    Some((Uint256::from(arr), seq))
                                } else { None }
                            })
                        {
                            last_validated_target = Some((hash, seq));
                        }
                    }
                }

                let mut persisted_validated_bootstrap_target = last_validated_target;
                let mut acquiring_consensus_ledger: Option<Uint256> = None;
                let mut last_check_accept_at = Instant::now();
                // History sync planner state — persists across loop iterations.
                // Drives fetch-pack requests and prefetch planning.
                let mut history_sync_state = ledger::history_sync::LedgerHistorySyncState::<
                    std::sync::Arc<ledger::Ledger>,
                > {
                    complete_ledgers: basics::range_set::RangeSet::new(),
                    fetch_state: ledger::history_fetch::FetchForHistoryState {
                        fetch_seq: 0,
                        fill_in_progress: 0,
                        hist_ledger: None,
                    },
                    fetch_pack_issued_at: None,
                };
                let mut last_ping_at = Instant::now();
                let mut known_endpoints: HashMap<std::net::SocketAddr, KnownEndpoint> =
                    HashMap::new();
                let mut redirect_bootcache = peerfinder_bootcache_path
                    .as_deref()
                    .map(load_peerfinder_bootcache)
                    .unwrap_or_default();
                let mut bootcache_dirty = false;
                let mut recent_autoconnect_attempts: HashMap<std::net::IpAddr, Instant> =
                    HashMap::new();
                let (bootcache_tx, bootcache_rx) =
                    std::sync::mpsc::channel::<PeerfinderBootcacheEvent>();
                let loaded_ledger_runtime = app::AppLoadedLedgerRuntime::from_root(&app);

                // Direct ledger_data channel from overlay to acquisition router.
                // the network thread. This channel replicates that: the overlay
                // sends TmLedgerData here as soon as it arrives, and a dedicated
                // router thread dispatches to acquisition threads without waiting
                // for the catchup loop's snapshot cycle.
                let (_ledger_data_tx, ledger_data_rx) =
                    std::sync::mpsc::channel::<overlay::PeerMessage<overlay::TmLedgerData>>();
                // We'll register this with the overlay once it's available.
                let ledger_data_rx = Arc::new(Mutex::new(Some(ledger_data_rx)));
                let mut overlay_channel_registered = false;
                let direct_router_counter = Arc::new(AtomicU64::new(0));

                // Channel for checkAccept to send validated hashes to the
                // catchup loop, matching reference InboundLedgers::acquire pattern.
                let (validated_hash_tx, validated_hash_rx) =
                    std::sync::mpsc::channel::<(Uint256, u32)>();

                // Spawn a dedicated validation processing thread.
                let val_app = app.clone();
                let val_state = Arc::clone(&state);
                let val_accept_sink = CheckAcceptSink {
                    app: val_app.clone(),
                    validated_tx: validated_hash_tx,
                };
                // Create a bounded notify channel so the validation thread
                // wakes instantly when a validation arrives from the overlay.
                // Buffer of 1: multiple arrivals between wakes collapse into
                // one signal, which is fine — we drain the full queue each wake.
                let (val_notify_tx, val_notify_rx) = std::sync::mpsc::sync_channel::<()>(1);
                // validation_notify is set by the bootstrap validation-processor thread.
                // Don't override it here.
                let _ = thread::Builder::new()
                    .name("xrpld-validation-processor".to_owned())
                    .spawn(move || {
                        // Elevate thread priority — consensus must never be starved by RPC load.
                        set_consensus_thread_priority();

                        // call got_tx_set when TX sets are acquired from peers.
                        let map_complete_rx = val_app.consensus_runtime()
                            .and_then(|cr| cr.take_map_complete_receiver());

                        while !val_state.stop.load(Ordering::Acquire) {
                            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            // Process manifests from peers — needed for validator key rotation
                            if let Some(overlay_runtime) = val_app.overlay_runtime() {
                                let snapshot = overlay_runtime.overlay().queued_inbound().take_snapshot();
                                for peer_msg in &snapshot.manifests {
                                    for manifest_blob in &peer_msg.message.list {
                                        if let Some(manifest) = app::deserialize_manifest(&manifest_blob.stobject) {
                                            let _ = val_app.manifest_cache().apply_manifest(manifest);
                                        }
                                    }
                                }
                                // Re-queue validations that were in the snapshot
                                overlay_runtime.overlay().queued_inbound().requeue_validations(snapshot.validations);
                            }
                            // In --start mode, the bootstrap thread exclusively
                            // processes validations. Skip here to avoid races.
                            if !val_app.need_network_ledger() {
                                process_queued_validations(&val_app, &val_accept_sink);
                            }

                            if !val_app.need_network_ledger()
                            && let Some(rx) = &map_complete_rx {
                                while let Ok((hash, set)) = rx.try_recv() {
                                    if let (Some(consensus_runtime), Some(network_ops_runtime)) =
                                        (val_app.consensus_runtime(), val_app.network_ops_runtime())
                                    {
                                        network_ops_runtime.handle_map_complete(
                                            consensus_runtime.as_ref(),
                                            hash,
                                            set,
                                        );
                                    }
                                }
                            }

                            // Process peer proposals — feed to consensus engine (only after sync)
                            if !val_app.need_network_ledger()
                            && let Some(overlay_runtime) = val_app.overlay_runtime() {
                                let proposals = overlay_runtime.overlay().take_proposals();
                                if !proposals.is_empty()
                                    && let (Some(consensus_runtime), Some(_network_ops_runtime)) =
                                        (val_app.consensus_runtime(), val_app.network_ops_runtime())
                                    {
                                        for proposal in proposals {
                                            let close_time = val_app.shared_time_keeper().close_time();
                                            let prop = consensus::ConsensusProposal::new(
                                                proposal.previous_ledger,
                                                proposal.message.propose_seq,
                                                proposal.current_tx_hash,
                                                close_time,
                                                close_time,
                                                proposal.public_key,
                                            );
                                            consensus_runtime.push_proposal(app::runtime::component_runtime::PendingProposal {
                                                now: close_time,
                                                public_key: proposal.public_key,
                                                signature: proposal.message.signature.clone(),
                                                suppression_id: proposal.suppression,
                                                proposal: prop,
                                            });
                                        }
                                    }
                            }
                            // Drive consensus state machine — but only after initial sync.
                            // During cold bootstrap, timer_tick panics because the consensus
                            // tokio::sync::Mutex was created on the main runtime which is
                            // Consensus timer is driven exclusively by the bootstrap loop.
                            if !val_app.need_network_ledger()
                                && let Some(_consensus_runtime) = val_app.consensus_runtime() {
                                    let tick_start = Instant::now();
                                    let latency_ms = tick_start.elapsed().as_millis() as u64;
                                    val_app.set_status_rpc_io_latency_ms(Some(latency_ms));
                                }
                            })); // end catch_unwind
                            // Wait for a validation to arrive (instant wake) or
                            // fall through after 500ms for proposal/timer work.
                            let _ = val_notify_rx.recv_timeout(Duration::from_millis(200));
                        }
                    });

                while !state.stop.load(Ordering::Acquire) {
                    let Some(overlay_runtime) = app.overlay_runtime() else {
                        thread::sleep(Duration::from_secs(2));
                        continue;
                    };

                    // Register direct ledger_data channel with overlay (once)
                    // and spawn a dedicated router thread that immediately
                    // dispatches TmLedgerData to acquisition threads, matching
                    if !overlay_channel_registered {
                        let direct_registry = Arc::clone(&acq_registry);
                        let dc = Arc::clone(&direct_router_counter);
                        overlay_runtime.overlay().queued_inbound()
                            .set_ledger_data_router(Box::new(move |peer_id, message| {
                                dc.fetch_add(1, Ordering::Relaxed);
                                if let Some((hash, packet)) = parse_ledger_data_packet(&message) {
                                    let hash = *hash.as_uint256();
                                    route_ledger_data_to_acq(
                                        &direct_registry,
                                        &hash,
                                        peer_id as u64,
                                        packet,
                                    );
                                }
                            }));
                        overlay_channel_registered = true;
                    }

                    // === reference OverlayImpl::Timer (every 1 second) ===
                    let peers = overlay_runtime.overlay().active_peers();

                    // sendEndpoints (reference PeerFinder::Logic::buildEndpointsForPeers)
                    if last_endpoints_at.elapsed() >= PEERFINDER_SECONDS_PER_MESSAGE {
                        last_endpoints_at = Instant::now();
                        prune_known_endpoints(&mut known_endpoints, Instant::now());
                        let listening_port = overlay_runtime.listener_setup().map(|setup| setup.port);
                        for peer in &peers {
                            let endpoints_v2 = build_endpoint_broadcast(
                                listening_port,
                                &known_endpoints,
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
                        prune_known_endpoints(&mut known_endpoints, now);
                        prune_recent_connect_attempts(&mut recent_autoconnect_attempts, now);
                        while let Ok(event) = bootcache_rx.try_recv() {
                            match event {
                                PeerfinderBootcacheEvent::Redirects(peers) => {
                                    let mut added = 0usize;
                                    for addr in peers.into_iter().take(PEERFINDER_MAX_REDIRECTS) {
                                        if insert_peerfinder_bootcache(&mut redirect_bootcache, addr)
                                        {
                                            bootcache_dirty = true;
                                            added += 1;
                                        }
                                    }
                                    if added > 0 {
                                        tracing::debug!(target: "peerfinder", added, total = redirect_bootcache.len(), "Redirect bootcache updated");
                                    }
                                }
                                PeerfinderBootcacheEvent::Success(addr) => {
                                    peerfinder_bootcache_success(&mut redirect_bootcache, addr);
                                    bootcache_dirty = true;
                                }
                                PeerfinderBootcacheEvent::Failure(addr) => {
                                    peerfinder_bootcache_failure(&mut redirect_bootcache, addr);
                                    bootcache_dirty = true;
                                }
                            }
                        }
                        if bootcache_dirty
                            && last_bootcache_save_at.elapsed()
                                >= PEERFINDER_BOOTCACHE_UPDATE_COOLDOWN
                        {
                            if let Some(path) = peerfinder_bootcache_path.as_deref() {
                                save_peerfinder_bootcache(path, &redirect_bootcache);
                            }
                            bootcache_dirty = false;
                            last_bootcache_save_at = Instant::now();
                        }
                        let target_outbound_peers = peerfinder_outbound_target(
                            overlay_runtime.overlay().limit(),
                            overlay_runtime.listener_setup().is_some(),
                        );
                        let active_outbound_peers =
                            overlay_runtime.overlay().active_outbound_peers_count();
                        if peers.len() < target_outbound_peers {
                            tracing::debug!(target: "peerfinder", peers = peers.len(), outbound = active_outbound_peers, target_outbound = target_outbound_peers, pending = overlay_runtime.overlay().pending_outbound_attempts(), known_endpoints = known_endpoints.len(), "Peer count below target");
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
                                &known_endpoints,
                                &recent_autoconnect_attempts,
                                now,
                            );
                            let selected = if selected.is_empty() {
                                select_bootcache_endpoints(
                                    &connected_addrs,
                                    &redirect_bootcache,
                                    &recent_autoconnect_attempts,
                                    now,
                                )
                            } else {
                                selected
                            };
                            if !selected.is_empty() {
                                tracing::debug!(target: "peerfinder", selected = selected.len(), active_outbound = active_outbound_peers, target_outbound = target_outbound_peers, known_endpoints = known_endpoints.len(), bootcache = redirect_bootcache.len(), "Autoconnect selected");
                            }
                            for addr in selected {
                                if active_outbound_peers + scheduled_attempts
                                    >= target_outbound_peers
                                {
                                    break;
                                }
                                connected_addrs.insert(peerfinder_canonical_ip(addr.ip()));
                                recent_autoconnect_attempts.insert(
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
                                                    // Start the peer session read/write loop
                                                    if let Some(session) = result.session.take() {
                                                        overlay.spawn_peer_session(std::sync::Arc::clone(&result.peer), session);
                                                    }
                                                    let _ = bootcache_tx
                                                        .send(PeerfinderBootcacheEvent::Success(addr));
                                                }
                                                Err(overlay::ConnectAttemptError::Redirect(peers)) => {
                                                    tracing::debug!(target: "peerfinder", %addr, redirect_count = peers.len(), "Autoconnect redirected");
                                                    let _ = bootcache_tx
                                                        .send(PeerfinderBootcacheEvent::Redirects(peers));
                                                }
                                                Err(error) => {
                                                    tracing::debug!(target: "peerfinder", %addr, %error, "Autoconnect failed");
                                                    let _ = bootcache_tx
                                                        .send(PeerfinderBootcacheEvent::Failure(addr));
                                                }
                                            }
                                        }
                                    });
                            }
                        }
                    }

                    // === reference PeerImp::onTimer (every 60 seconds): send ping ===
                    if last_ping_at.elapsed() >= Duration::from_secs(60) {
                        last_ping_at = Instant::now();
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
                        for p in &peers {
                            p.send(wire.clone());
                        }
                        overlay_runtime.overlay().delete_idle_peers();
                    }

                    if peers.is_empty() {
                        thread::sleep(Duration::from_secs(2));
                        continue;
                    }

                    // gotFetchPack: if the bootstrap/consensus loop stored
                    // fetch-pack data, signal ALL active InboundLedger workers
                    // to re-check local storage immediately (matching rippled
                    // InboundLedgers::gotFetchPack → trigger checkLocal on all).
                    if app.take_fetch_pack_ready() {
                        let registry_guard = acq_registry.lock().expect("acq registry");
                        for tx in registry_guard.values() {
                            let _ = tx.send(AcqMsg::FetchPackReady);
                        }
                    }

                    let validated = app.ledger_master_runtime()
                        .map(|lm| lm.ledger_master().valid_ledger_seq())
                        .unwrap_or(0);
                    let current_hash = app.ledger_master_runtime()
                        .and_then(|lm| lm.ledger_master().validated_ledger())
                        .map(|ledger| *ledger.header().hash.as_uint256());

                    let peer_ranges = peers
                        .iter()
                        .map(|peer| peer.ledger_range())
                        .filter(|(min, max)| *min > 0 && *max >= *min)
                        .collect::<Vec<_>>();
                    let peer_max_seq = peer_ranges.iter().map(|(_, max)| *max).max().unwrap_or(0);
                    let peer_min_seq = peer_ranges.iter().map(|(min, _)| *min).min().unwrap_or(0);
                    let shared_min_seq = peer_ranges.iter().map(|(min, _)| *min).max().unwrap_or(0);
                    let shared_max_seq = peer_ranges.iter().map(|(_, max)| *max).min().unwrap_or(0);
                    let has_shared_range = shared_min_seq != 0
                        && shared_max_seq != 0
                        && shared_min_seq <= shared_max_seq;
                    let selection_ceiling = if has_shared_range {
                        shared_max_seq
                    } else {
                        peer_max_seq
                    };
                    let selection_floor = if has_shared_range { shared_min_seq } else { 0 };

                    if validated <= 1 && selection_floor > 1 {
                        inbound_ledgers.remove_in_progress_below_seq(selection_floor);
                    }

                    let target_seq = select_target_seq(
                        validated,
                        has_shared_range,
                        selection_ceiling,
                        last_validated_target.map(|(_, seq)| seq),
                    );

                    if last_diag_at.elapsed() >= Duration::from_secs(10) {
                        let peers_connected = peers.len();
                        let validated_seq = validated;
                        let mode = format!("{:?}", app.network_ops_operating_mode());
                        tracing::debug!(target: "main", peers_connected, validated_seq, %mode, "Status heartbeat");
                        tracing::debug!(target: "catchup", validated, peers = peers.len(), peer_min_seq, peer_max_seq, shared_min_seq = if has_shared_range { shared_min_seq } else { 0 }, shared_max_seq = if has_shared_range { shared_max_seq } else { 0 }, target_seq, direct_router_calls = direct_router_counter.load(Ordering::Relaxed), "Catchup status");
                        last_diag_at = Instant::now();
                        // Milestone summary
                        let elapsed = catchup_start.elapsed().as_secs();
                        let node_store_mb = node_store_usage_path
                            .as_ref()
                            .map(|path| path_size_bytes(path.as_path()) / (1024 * 1024))
                            .unwrap_or(0);
                        tracing::debug!(target: "catchup", elapsed, node_db_mib = node_store_mb, acqs = inbound_ledgers.active_count(), first_ledger = milestone_first_ledger.map(|d| format!("{:.1}s", d.as_secs_f64())).unwrap_or_else(|| "pending".to_owned()), inbound_info = inbound_ledgers.info_summary(), "Milestone");
                        if validated > 1 && target_seq > validated.saturating_add(1) && inbound_ledgers.active_count() == 0 {
                            tracing::info!(target: "main", "Ledger sync stalled — no progress");
                        }
                    }

                    if last_cache_sweep_at.elapsed() >= Duration::from_secs(15) {
                        last_cache_sweep_at = Instant::now();
                        use shamap::family::FullBelowCache;

                        shared_tree_cache.sweep();
                        shared_full_below.sweep();
                        shared_fetch_pack.sweep();
                        shared_stored.sweep();

                        if let Some(ledger_master_runtime) = app.ledger_master_runtime() {
                            ledger_master_runtime.ledger_master().sweep();
                        }
                        if let Some(node_family) = app.node_family() {
                            node_family.sweep();
                        }
                    }
                    // Consume validated hashes from checkAccept. reference keeps a
                    // set of active InboundLedger objects, so keep several
                    // validated targets alive instead of replacing the only
                    // in-progress acquisition.
                    let mut validated_hash_targets = Vec::new();
                    while let Ok((hash, seq)) = validated_hash_rx.try_recv() {
                        // highest quorum-backed candidate ledger separately from
                        // the individual inbound acquire calls below.
                        if seq > last_validated_target.map_or(0, |(_, s)| s)
                            || (seq == 0 && last_validated_target.is_none_or(|(h, _)| h != hash))
                        {
                            tracing::debug!(target: "bootstrap", seq, hash = %debug_hash8(&hash), "last_validated_target set");
                            last_validated_target = Some((hash, seq));
                        }
                        if Some(hash) != current_hash && (seq == 0 || seq > validated) {
                            validated_hash_targets.push((hash, seq));
                            if seq > 1 && validated <= 1 {
                                // During cold bootstrap, eagerly acquire
                                // quorum-backed validated hashes, but keep the
                                // number of live inbound acquisitions within
                                // the same node-size LedgerFetch budget reference
                                // uses for generic ledger fetch work.
                                if bootstrap_acquire_budget_available(
                                    validated,
                                    inbound_ledgers.active_count(),
                                    catchup_profile.ledger_fetch_limit,
                                    inbound_ledgers.contains(&hash),
                                ) {
                                    inbound_ledgers.acquire(hash, seq, validated);
                                    inbound_ledgers.send_peers(&peers);
                                }
                            }
                        }
                        // check if we already have that ledger and can accept it.
                        if seq > validated
                            && let Some(lm) = app.ledger_master_runtime() {
                                let ledger_master = lm.ledger_master();
                                let ledger = ledger_master.get_ledger_by_hash(
                                    basics::sha_map_hash::SHAMapHash::new(hash),
                                );
                                if ledger.is_none() {
                                    if bootstrap_acquire_budget_available(
                                        validated,
                                        inbound_ledgers.active_count(),
                                        catchup_profile.ledger_fetch_limit,
                                        inbound_ledgers.contains(&hash),
                                    ) {
                                        inbound_ledgers.acquire(hash, seq, validated);
                                    }
                                    continue;
                                }
                                let ledger = ledger.unwrap();
                                if !ledger_master.check_accept_ledger(
                                    ledger.as_ref(),
                                    0,
                                    0,
                                    app.current_close_time_seconds(),
                                ) {
                                    continue;
                                }
                                let validations = app.validations().store()
                                    .trusted_for_ledger_by_sequence(hash, seq);
                                let val_count = app.validators()
                                    .negative_unl_filter_validations(validations).len();
                                let needed = if app.standalone() { 0 } else { app.validators().quorum() };
                                if val_count >= needed {
                                    let promoted = try_promote_ledger_with_validations(
                                        &app,
                                        &ledger_master,
                                        std::sync::Arc::clone(&ledger),
                                        val_count,
                                        needed,
                                        false,
                                    );
                                    if promoted {
                                        advance_published_ledgers_after_validation(
                                            &app,
                                            &peers,
                                            &mut inbound_ledgers,
                                            ledger,
                                        );
                                    }
                                    tracing::info!(target: "ledger_master", seq, val_count, "checkAccept accepted ledger");
                                }
                            }
                    }
                    // enough validations, even before a ledger is fully validated.
                    // This enables endpoint discovery and auto-connect.
                    if !validated_hash_targets.is_empty() && validated <= 1
                        && let Some((_, seq)) = validated_hash_targets.last() {
                            overlay_runtime.overlay().check_tracking(*seq);
                        }
                    // --- 1-second checkAccept timer (compatibility safety net) ---
                    // If the highest-validated hash is not yet accepted, periodically
                    // re-check whether the ledger has appeared in history (e.g. via
                    // acquisition completing between ticks).
                    if last_check_accept_at.elapsed() >= Duration::from_secs(1) {
                        last_check_accept_at = Instant::now();
                        if let Some((hash, seq)) = last_validated_target {
                            let is_candidate = seq > validated || seq == 0;
                            if is_candidate {
                                // If seq=0 (no sfLedgerSequence), acquire by hash directly
                                // using validated+1 as a seq hint (hash is the primary key)
                                if seq == 0 {
                                    let seq_hint = validated.saturating_add(1).max(2);
                                    if bootstrap_acquire_budget_available(
                                        validated,
                                        inbound_ledgers.active_count(),
                                        catchup_profile.ledger_fetch_limit,
                                        inbound_ledgers.contains(&hash),
                                    ) {
                                        inbound_ledgers.acquire(hash, seq_hint, validated);
                                    }
                                } else if !inbound_ledgers.is_in_progress(&hash)
                                    && bootstrap_acquire_budget_available(
                                        validated,
                                        inbound_ledgers.active_count(),
                                        catchup_profile.ledger_fetch_limit,
                                        inbound_ledgers.contains(&hash),
                                    )
                                {
                                    inbound_ledgers.acquire(hash, seq, validated);
                                }
                                if let Some(lm) = app.ledger_master_runtime() {
                                    let ledger_master = lm.ledger_master();
                                    if let Some(ledger) = ledger_master.get_ledger_by_hash(
                                        basics::sha_map_hash::SHAMapHash::new(hash),
                                    ) {
                                        if !ledger_master.check_accept_ledger(
                                            ledger.as_ref(),
                                            0,
                                            0,
                                            app.current_close_time_seconds(),
                                        ) {
                                            // canBeCurrent failed — don't clear target, retry next tick
                                        } else {
                                            let validations = app.validations().store()
                                                .trusted_for_ledger_by_sequence(hash, seq);
                                            let val_count = app.validators()
                                                .negative_unl_filter_validations(validations).len();
                                            let needed = if app.standalone() { 0 } else { app.validators().quorum() };
                                            if val_count >= needed {
                                                let promoted = try_promote_ledger_with_validations(
                                                    &app,
                                                    &ledger_master,
                                                    std::sync::Arc::clone(&ledger),
                                                    val_count,
                                                    needed,
                                                    false,
                                                );
                                                if promoted {
                                                    advance_published_ledgers_after_validation(
                                                        &app,
                                                        &peers,
                                                        &mut inbound_ledgers,
                                                        ledger,
                                                    );
                                                }
                                                tracing::info!(target: "ledger_master", seq, val_count, "checkAccept-timer accepted ledger");
                                                last_validated_target = None;
                                            }
                                        }
                                    }
                                }
                            } else {
                                // Already accepted — clear target.
                                last_validated_target = None;
                            }
                        }
                    }

                    // --- storeLedger equivalent (rippled InboundLedger::done → storeLedger) ---
                    // Insert completed InboundLedger results into LedgerHistory
                    // so that acquire_consensus_ledger (get_ledger_by_hash) can
                    // find them when the consensus engine needs to switch LCL.
                    {
                        let store_results = inbound_ledgers.poll_results();
                        if !store_results.is_empty()
                            && let Some(lm_rt) = app.ledger_master_runtime() {
                                for (_hash, ledger, _skip) in &store_results {
                                    let stored = std::sync::Arc::new(ledger.clone());
                                    lm_rt.ledger_master().ledger_history().insert(
                                        std::sync::Arc::clone(&stored),
                                        false,
                                    );
                                }
                            }
                    }

                    // Trigger parallel InboundLedger acquisitions for ALL unique
                    // peer hashes we don't have (matching rippled which manages
                    // many concurrent InboundLedgers). This enables catching up
                    // at the rate peers advance rather than one-at-a-time.
                    if let Some(lm_rt) = app.ledger_master_runtime()
                        && let Some(overlay_rt) = app.overlay_runtime() {
                            use overlay::Overlay as _;
                            let our_hash = lm_rt.ledger_master().closed_ledger()
                                .map(|l| *l.header().hash.as_uint256())
                                .unwrap_or_default();
                            let peers = overlay_rt.overlay().active_peers();
                            // Acquire ALL unique peer hashes we don't already have
                            let mut triggered = 0usize;
                            for p in peers.iter() {
                                let h = p.closed_ledger_hash();
                                if !h.is_zero() && h != our_hash
                                    && !inbound_ledgers.contains(&h)
                                {
                                    inbound_ledgers.acquire(h, 0, 0);
                                    triggered += 1;
                                }
                            }
                            if triggered > 0 {
                                inbound_ledgers.send_peers(&peers);

                                // Send fetch pack request for instant state delta.
                                // Per rippled's protocol: send the hash of a ledger the
                                // PEER has. They diff it against its parent and send back
                                // the state delta nodes we need.
                                if let Some(lm_rt) = app.ledger_master_runtime()
                                    && let Some(closed) = lm_rt.ledger_master().closed_ledger() {
                                        let our_hash = closed.header().hash;
                                        // Use the peer's closed_ledger_hash (they have it)
                                        for p in peers.iter() {
                                            let peer_hash = p.closed_ledger_hash();
                                            if !peer_hash.is_zero() && peer_hash != *our_hash.as_uint256() {
                                                let fp_msg = ledger::make_fetch_pack_request(
                                                    basics::sha_map_hash::SHAMapHash::new(peer_hash)
                                                );
                                                let wire = overlay::Message::new(fp_msg, None);
                                                p.send(wire);
                                                tracing::info!(target: "consensus",
                                                    our_seq = closed.header().seq,
                                                    "Fetch pack request sent (peer hash)"
                                                );
                                                break;
                                            }
                                        }
                                    }
                            }
                        }

                    if target_seq > 1 {
                        // --- Persistent tick-based acquisition (reference InboundLedger parity) ---
                        // Maintain persistent InboundLedgerLocal owners that
                        // accumulate state across 3s ticks, matching the reference
                        // InboundLedgers cache shape.

                        let ledger_master_runtime = app.ledger_master_runtime();

                        // Drain completed/failed entries before trying to acquire.
                        // later promotion/advance checks observe the cache.
                        let early_results = inbound_ledgers.poll_results();
                        if acquiring_consensus_ledger
                            .is_some_and(|hash| !inbound_ledgers.is_in_progress(&hash))
                        {
                            acquiring_consensus_ledger = None;
                        }

                        let target_hash = hash_for_seq_from_available_sources(
                            target_seq,
                            &inbound_ledgers,
                            history_sync_state.fetch_state.hist_ledger.as_ref(),
                            app.validated_ledger().as_ref(),
                            loaded_ledger_runtime.as_ref(),
                        );
                        let target_reference = if target_hash.is_none() {
                            candidate_reference_hash_from_available_sources(
                                target_seq,
                                &inbound_ledgers,
                                history_sync_state.fetch_state.hist_ledger.as_ref(),
                                app.validated_ledger().as_ref(),
                                loaded_ledger_runtime.as_ref(),
                            )
                        } else {
                            None
                        };

                        let mut maybe_start_acq = |seq: u32, hash: Uint256| {
                            if seq <= 1 { return; }
                            if !bootstrap_acquire_budget_available(
                                validated,
                                inbound_ledgers.active_count(),
                                catchup_profile.ledger_fetch_limit,
                                inbound_ledgers.contains(&hash),
                            ) {
                                // During cold bootstrap, keep the total number
                                // of live inbound acquisitions within the same
                                // node-size LedgerFetch budget that reference
                                // LedgerMaster uses for generic fetch work.
                                return;
                            }
                            if ledger_master_runtime
                                .as_ref()
                                .is_some_and(|runtime| runtime.ledger_master().have_ledger(seq))
                            { return; }
                            inbound_ledgers.acquire(hash, seq, validated);
                            // Send current peers right after spawning so the acquisition
                            // doesn't start with an empty peer set.
                            inbound_ledgers.send_peers(&peers);
                        };

                        if validated > 1 {
                            persisted_validated_bootstrap_target = None;
                        }

                        if validated <= 1 {
                            if let Some((hash, seq)) = cold_bootstrap_persisted_validated_target(
                                validated,
                                persisted_validated_bootstrap_target,
                            ) {
                                full_sync_debug!(
                                    "[full_debug][bootstrap_anchor] acquire_persisted seq={} hash={} live_last={}",
                                    seq,
                                    debug_hash8(&hash),
                                    last_validated_target
                                        .map(|(_, live_seq)| live_seq.to_string())
                                        .unwrap_or_else(|| "none".to_owned())
                                );
                                maybe_start_acq(seq, hash);
                            } else if let Some(target_hash) = target_hash {
                                maybe_start_acq(target_seq, *target_hash.as_uint256());
                            } else if let Some((reference_seq, reference_hash)) = target_reference {
                                maybe_start_acq(reference_seq, *reference_hash.as_uint256());
                            }
                        }

                        if let Some((hash, seq)) =
                            select_consensus_acquisition_target(validated, &validated_hash_targets)
                            && seq > 0 {
                                // does not continually switch to every new
                                // validation hash while one consensus ledger is
                                // already being acquired.
                                if acquiring_consensus_ledger.is_none()
                                    || acquiring_consensus_ledger == Some(hash)
                                {
                                    maybe_start_acq(seq, hash);
                                    if inbound_ledgers.is_in_progress(&hash) {
                                        acquiring_consensus_ledger = Some(hash);
                                    }
                                }
                            }
                            // seq=0: handled by 1-second checkAccept timer via last_validated_target

                        inbound_ledgers.send_peers(&peers);

                        // === OVERLAY DUTIES (always, matching reference OverlayImpl::Timer) ===
                        // Take overlay snapshot once per loop iteration.
                        // PeerImp::onMessage handlers. We consolidate here.
                        {
                            let snapshot = overlay_runtime.overlay().take_queued_inbound_snapshot();
                            overlay_runtime
                                .overlay()
                                .requeue_validations(snapshot.validations);

                            // Accumulate discovered endpoints for auto-connect
                            let now = Instant::now();
                            for batch in &snapshot.endpoints {
                                for ep in &batch.endpoints {
                                    // mtENDPOINTS into both livecache and bootcache. The
                                    // live cache intentionally expires after 30s, while
                                    // bootcache keeps usable candidates for later
                                    // autoconnect rounds.
                                    if insert_peerfinder_bootcache(
                                        &mut redirect_bootcache,
                                        ep.endpoint,
                                    ) {
                                        bootcache_dirty = true;
                                    }
                                    remember_known_endpoint(
                                        &mut known_endpoints,
                                        ep.endpoint,
                                        ep.hops,
                                        now,
                                    );
                                }
                            }

                            // --- Route TmLedgerData to acquisitions ---
                            // Use direct channel (immediate from overlay) plus
                            // snapshot (for any that arrived before channel was
                            // registered). reference gotLedgerData is called directly
                            // from the network thread for immediate processing.
                            let mut direct_messages = Vec::new();
                            let mut direct_channel_capped = false;
                            for _ in 0..MAX_DIRECT_LEDGER_DATA_PER_TICK {
                                match ledger_data_rx
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

                            // Process direct channel messages first (faster path)
                            for message in &direct_messages {
                                if let Some(cookie) = message.message.request_cookie {
                                    if let Some(target) = peers.iter().find(|p| p.id() == cookie) {
                                        let mut fwd = message.message.clone();
                                        fwd.request_cookie = None;
                                        let reply = overlay::ProtocolMessage::new(
                                            overlay::ProtocolPayload::LedgerData(fwd),
                                        );
                                        target.send(overlay::Message::new(reply, None));
                                    }
                                    continue;
                                }
                                if let Some((hash, packet)) =
                                    parse_ledger_data_packet(&message.message)
                                {
                                    let hash = *hash.as_uint256();
                                    if route_ledger_data_to_acq(&acq_registry, &hash, message.peer_id as u64, packet.clone()) {
                                        routed += 1;
                                    } else {
                                        unrouted += 1;
                                        // into the shared fetch-pack cache. This is free
                                        // future progress. reference the reference source:209.
                                        if message.message.r#type == 2
                                            && let Some((_, packet)) = parse_ledger_data_packet(&message.message) {
                                                let mut fp_store = SharedFetchPack::new(Arc::clone(&shared_fetch_pack));
                                                let _ = ledger::stash_stale_packet(&packet, &mut fp_store);
                                            }
                                    }
                                }
                            }

                            // Also process snapshot ledger_data (fallback path)
                            for message in &snapshot.ledger_data {
                                // to the peer identified by the cookie.
                                if let Some(cookie) = message.message.request_cookie {
                                    if let Some(target) = peers.iter().find(|p| p.id() == cookie) {
                                        let mut fwd = message.message.clone();
                                        fwd.request_cookie = None;
                                        let reply = overlay::ProtocolMessage::new(
                                            overlay::ProtocolPayload::LedgerData(fwd),
                                        );
                                        target.send(overlay::Message::new(reply, None));
                                    }
                                    continue;
                                }
                                if let Some((hash, packet)) =
                                    parse_ledger_data_packet(&message.message)
                                {
                                    let hash = *hash.as_uint256();
                                    if route_ledger_data_to_acq(&acq_registry, &hash, message.peer_id as u64, packet.clone()) {
                                        routed += 1;
                                    } else {
                                        unrouted += 1;
                                        if message.message.r#type == 2
                                            && let Some((_, packet)) = parse_ledger_data_packet(&message.message) {
                                                let mut fp_store = SharedFetchPack::new(Arc::clone(&shared_fetch_pack));
                                                let _ = ledger::stash_stale_packet(&packet, &mut fp_store);
                                            }
                                    }
                                }
                            }

                            // --- Route TMGetObjectByHash responses ---
                            for message in &snapshot.get_objects {
                                if message.message.query { continue; }
                                let ledger_hash = match message.message.ledger_hash.as_deref()
                                    .and_then(Uint256::from_slice)
                                {
                                    Some(h) => h,
                                    None => continue,
                                };
                                let packet_type = match message.message.r#type {
                                    3 => ledger::InboundLedgerDataType::TransactionNode,
                                    4 => ledger::InboundLedgerDataType::StateNode,
                                    6 => {
                                        // otFETCH_PACK: reference adds each object to the
                                        // shared fetch-pack cache via addFetchPack, then
                                        // calls gotFetchPack which triggers checkLocal on
                                        // all active InboundLedgers.
                                        for obj in &message.message.objects {
                                            let Some(hash_bytes) = obj.hash.as_deref() else { continue };
                                            let Some(hash) = Uint256::from_slice(hash_bytes) else { continue };
                                            let Some(data) = obj.data.as_ref() else { continue };
                                            // Add to shared cache (reference addFetchPack)
                                            shared_fetch_pack.add_fetch_pack(hash, data.clone());
                                        }
                                        // active acquisitions immediately so they find the
                                        // new data in the shared cache via AccountStateSF::getNode.
                                        for tx in acq_registry.lock().expect("acq registry").values() {
                                            let _ = tx.send(AcqMsg::FetchPackReady);
                                        }
                                        tracing::debug!(target: "inbound_ledger", objects = message.message.objects.len(), acquisitions = inbound_ledgers.active_count(), "Fetch-pack ingested");
                                        continue;
                                    }
                                    _ => continue,
                                };
                                let nodes: Vec<_> = message.message.objects.iter()
                                    .filter_map(|obj| {
                                        let data = obj.data.as_ref()?;
                                        Some(ledger::InboundLedgerNodeData::new(
                                            obj.node_id.clone(), data.clone(),
                                        ))
                                    })
                                    .collect();
                                if nodes.is_empty() { continue; }
                                let packet = ledger::InboundLedgerPacket::new(packet_type, nodes);
                                route_ledger_data_to_acq(&acq_registry, &ledger_hash, message.peer_id as u64, packet);
                            }

                            // --- Serve GetLedger requests (reference processLedgerRequest) ---
                            if let Some(loaded_ledger_runtime) = loaded_ledger_runtime.as_ref() {
                                for gl in &snapshot.get_ledgers {
                                    if let Some(peer) = peers.iter().find(|p| p.id() == gl.peer_id) {
                                        serve_get_ledger(
                                            loaded_ledger_runtime,
                                            &gl.message,
                                            peer.as_ref(),
                                            &peers,
                                        );
                                    }
                                }
                            }

                            if total_ledger_data > 0 || !snapshot.get_objects.is_empty()
                                || !snapshot.get_ledgers.is_empty()
                            {
                                tracing::debug!(target: "overlay", total_ledger_data, direct_channel_capped, routed, unrouted, get_ledgers = snapshot.get_ledgers.len(), get_objects = snapshot.get_objects.len(), acqs = inbound_ledgers.active_count(), "Route summary");
                            }

                            // --- Process inbound transactions from peers (reference NetworkOPs::processTransaction) ---
                            if !snapshot.transactions.is_empty() {
                                for queued_tx in &snapshot.transactions {
                                    let mut serial = protocol::SerialIter::new(&queued_tx.message.raw_transaction);
                                    let st_tx = match std::panic::catch_unwind(
                                        std::panic::AssertUnwindSafe(|| {
                                            protocol::STTx::from_serial_iter(&mut serial)
                                        }),
                                    ) {
                                        Ok(tx) => tx,
                                        Err(_) => continue,
                                    };
                                    let st_tx = std::sync::Arc::new(st_tx);
                                    let mut transaction: app::SharedTransaction =
                                        std::sync::Arc::new(std::sync::Mutex::new(
                                            app::tx_queue::transaction::Transaction::new(
                                                std::sync::Arc::clone(&st_tx),
                                            ),
                                        ));
                                    if let Some(network_ops_runtime) = app.network_ops_runtime() {
                                        let _ = network_ops_runtime.process_transaction(
                                            &mut transaction,
                                            false,
                                            false,
                                            false,
                                            || {
                                                app.job_queue().dispatch_next_job();
                                                true
                                            },
                                            || {},
                                        );
                                    }
                                }
                            }
                        }

                        // --- Check acquisition results (reference InboundLedgers parity) ---
                        // Sweep stale entries on timer
                        if last_inbound_sweep_at.elapsed() >= Duration::from_secs(10) {
                            last_inbound_sweep_at = Instant::now();
                            inbound_ledgers.sweep();
                        }
                        // Poll completed acquisitions (includes early-drained results)
                        let mut all_results = early_results;
                        all_results.extend(inbound_ledgers.poll_results());

                        // Pure acquire-and-trust: insert into history, then advance sequentially.
                        let mut any_new = false;
                        let mut acquired_ledgers = Vec::<std::sync::Arc<ledger::Ledger>>::new();
                        for (_acq_hash, ledger, is_skip_state) in all_results {
                            let acq_seq = ledger.header().seq;
                            tracing::info!(target: "catchup", seq = acq_seq, "LEDGER ACQUIRED");
                            let ledger = app.ledger_with_node_fetcher(std::sync::Arc::new(ledger));

                            // For skip-state ledgers, build the full state locally
                            // from the parent ledger + transaction set (reference tracking mode).
                            let ledger = if is_skip_state {
                                if let Some(lm) = app.ledger_master_runtime() {
                                    let parent_hash = ledger.header().parent_hash;
                                    if let Some(parent) = lm.ledger_master().get_ledger_by_hash(parent_hash) {
                                        // Extract tx items from the acquired ledger's tx map
                                        let mut tx_items: Vec<(Vec<u8>, basics::base_uint::Uint256)> = Vec::new();
                                        let mut fetch = |_: basics::sha_map_hash::SHAMapHash| -> Option<basics::memory::intrusive_pointer::SharedIntrusive<shamap::nodes::tree_node::SHAMapTreeNode>> { None };
                                        let _ = ledger.tx_map().visit_leaves(&mut fetch, &mut |item: &shamap::item::SHAMapItem| {
                                            tx_items.push((item.data().to_vec(), item.key()));
                                        });
                                        match app::build_ledger_from_acquired_tx(
                                            parent.as_ref(),
                                            ledger.header(),
                                            &tx_items,
                                        ) {
                                            Some(built) => {
                                                tracing::info!(target: "catchup", seq = acq_seq, "Locally built ledger — hash verified ✓");
                                                std::sync::Arc::new(built)
                                            }
                                            None => {
                                                tracing::warn!(target: "catchup", seq = acq_seq, "LOCAL BUILD FAILED");
                                                ledger
                                            }
                                        }
                                    } else {
                                        ledger // no parent yet, use as-is
                                    }
                                } else {
                                    ledger
                                }
                            } else {
                                ledger
                            };

                            if let Some(lm) = app.ledger_master_runtime() {
                                let mut to_store = std::sync::Arc::clone(&ledger);
                                {
                                    let l = Arc::make_mut(&mut to_store);
                                    l.set_validated();
                                    l.set_full();
                                    l.set_immutable(false);
                                }
                                let ledger_master = lm.ledger_master();
                                ledger_master.ledger_history().insert(std::sync::Arc::clone(&to_store), true);
                                ledger_master.mark_ledger_complete(to_store.header().seq);

                                // Advance validated pointer sequentially — never skip.
                                // This prevents "failed to read parent" errors from gaps.
                                let mut next_seq = ledger_master.valid_ledger_seq().saturating_add(1);
                                while let Some(next_ledger) = ledger_master.ledger_history().get_cached_ledger_by_seq(next_seq) {
                                    let _ = ledger_master.set_valid_ledger(next_ledger, None, None);
                                    next_seq = ledger_master.valid_ledger_seq().saturating_add(1);
                                }

                                // Persist to disk asynchronously via the main tokio runtime
                                let persist_app = app.clone();
                                let persist_ledger = to_store;
                                let _ = rt_handle_persist.spawn(async move {
                                    let persistence = ledger::LedgerPersistence::new(std::sync::Arc::new(
                                        persist_app.build_ledger_persistence_runtime(),
                                    ));
                                    persistence.pend_save_validated(persist_ledger, true, true);
                                });
                            }
                            any_new = true;
                            acquired_ledgers.push(std::sync::Arc::clone(&ledger));

                            if milestone_first_ledger.is_none() {
                                let elapsed = catchup_start.elapsed();
                                milestone_first_ledger = Some(elapsed);
                                tracing::info!(target: "catchup", seq = acq_seq, elapsed_s = elapsed.as_secs_f64(), "First ledger milestone");
                            }
                        }

                        if any_new {
                            // This is handled by the validated_hash_rx path (check_accept sends hash)
                            // and the bootstrap block below for validated=0.

                            // Bootstrap/advance: try to accept a newly acquired ledger as validated.
                            // Runs when validated=0 (bootstrap) AND when validated>0 (advance).
                            {
                                if let Some(lm) = app.ledger_master_runtime() {
                                    let ledger_master = lm.ledger_master();

                                    // Check every completed acquisition plus the
                                    // current quorum-backed target, matching the
                                    // callback shape more closely than the old
                                    // "last acquired ledger only" shortcut.
                                    let mut candidates: Vec<(basics::sha_map_hash::SHAMapHash, u32, std::sync::Arc<ledger::Ledger>)> = Vec::new();
                                    let mut seen_hashes = std::collections::HashSet::<Uint256>::new();

                                    // Candidate 1: quorum-validated target (may not be acquired yet)
                                    if let Some((target_hash, target_seq)) = last_validated_target {
                                        if target_seq > 1 && bootstrap_acquire_budget_available(
                                            validated,
                                            inbound_ledgers.active_count(),
                                            catchup_profile.ledger_fetch_limit,
                                            inbound_ledgers.contains(&target_hash),
                                        ) {
                                            inbound_ledgers.acquire(target_hash, target_seq, 0);
                                        }
                                        if let Some(ledger) = ledger_master.get_ledger_by_hash(
                                            basics::sha_map_hash::SHAMapHash::new(target_hash)
                                        ) {
                                            seen_hashes.insert(target_hash);
                                            candidates.push((basics::sha_map_hash::SHAMapHash::new(target_hash), target_seq, ledger));
                                        }
                                    }

                                    // Candidate 2: every ledger completed in this
                                    // batch, not just the last one.
                                    for acq in &acquired_ledgers {
                                        let h = *acq.header().hash.as_uint256();
                                        let s = acq.header().seq;
                                        if seen_hashes.insert(h) {
                                            candidates.push((basics::sha_map_hash::SHAMapHash::new(h), s, std::sync::Arc::clone(acq)));
                                        }
                                    }

                                    candidates.sort_by_key(|(_, seq, _)| *seq);

                                    for (hash, seq, ledger) in candidates {
                                        let validations = app.validations().store()
                                            .trusted_for_ledger_by_sequence(*hash.as_uint256(), seq);
                                        let val_count = app.validators()
                                            .negative_unl_filter_validations(validations).len();
                                        let needed = if app.standalone() { 0 } else { app.validators().quorum() };
                                        // If val_count is 0 but this hash was previously seen with
                                        // quorum (stored in last_validated_target), trust it directly.
                                        // Validations may have been pruned by the time the ledger arrives.
                                        // Also trust if this is the next sequential ledger after validated
                                        // (parent chain trust — candidate 3 above).
                                        let is_parent_chain_trusted = if let Some(current_val) = ledger_master.validated_ledger() {
                                            ledger.header().seq == current_val.header().seq + 1
                                                && ledger.header().parent_hash == current_val.header().hash
                                        } else { false };
                                        let effective_count = if val_count == 0 {
                                            if last_validated_target.is_some_and(|(th, _)| th == *hash.as_uint256())
                                                || is_parent_chain_trusted
                                            {
                                                needed // treat as having quorum
                                            } else {
                                                0
                                            }
                                        } else {
                                            val_count
                                        };
                                        tracing::debug!(target: "bootstrap", seq = ledger.header().seq, val_count, effective_count, needed, "Candidate evaluation");
                                        if effective_count >= needed {
                                            let check_result = ledger_master.check_accept_ledger(
                                                ledger.as_ref(),
                                                effective_count,
                                                needed,
                                                app.current_close_time_seconds(),
                                            );
                                            tracing::debug!(target: "bootstrap", seq = ledger.header().seq, check_result, valid_ledger_seq = ledger_master.valid_ledger_seq(), "check_accept result");
                                            let promoted = try_promote_ledger_with_validations(
                                                &app, &ledger_master,
                                                std::sync::Arc::clone(&ledger),
                                                val_count, needed, false,
                                            );
                                            if promoted {
                                                tracing::info!(target: "bootstrap", seq = ledger.header().seq, "Accepted as validated ✓");

                                                // Start first consensus round once we have a validated ledger.
                                                if let (Some(cr), Some(network_ops_runtime)) =
                                                    (app.consensus_runtime(), app.network_ops_runtime())
                                                {
                                                    if network_ops_runtime
                                                        .maybe_begin_consensus_from_validated(
                                                            cr.as_ref(),
                                                            std::sync::Arc::clone(&ledger),
                                                        )
                                                    {
                                                        tracing::info!(target: "consensus", seq = ledger.header().seq, "First round started from validated");
                                                    } else {
                                                        // Consensus will pick up the latest
                                                        // validated ledger via end_consensus's
                                                        // consensus_previous_ledger() call.
                                                    }
                                                }

                                                advance_published_ledgers_after_validation(
                                                    &app, &peers, &mut inbound_ledgers,
                                                    ledger,
                                                );
                                            }
                                        }
                                    }

                                    if let Some((_, target_seq)) = last_validated_target
                                        && ledger_master.valid_ledger_seq() >= target_seq
                                    {
                                        last_validated_target = None;
                                    }
                                }
                            }

                            try_advance_catchup(&app, app.node_store().as_ref(), &peers, &mut inbound_ledgers);
                        }
                    }

                    // --- Cold bootstrap: sweep + re-acquire (C++ parity) ---
                    // C++ InboundLedger::onTimer fires every 3s. If wasProgress=false,
                    // increments timeouts_. After 6 consecutive no-progress ticks, the
                    // acquisition is marked failed and removed.
                    //
                    // During cold start we keep one-at-a-time (full state trees are
                    // large; parallel downloads waste bandwidth on near-identical data).
                    // The sweep ensures stale acquisitions (peers stopped responding)
                    // die after 6 consecutive no-progress checks, freeing the slot for
                    // the latest validated target. Never sweep acquisitions that have
                    // received substantial data — they're working, just slow in the tail.
                    if validated <= 1 {
                        const ACQUIRE_TIMEOUT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);
                        const ACQUIRE_TIMEOUT_RETRIES_MAX: u32 = 6;

                        let now = Instant::now();
                        let stale: Vec<Uint256> = inbound_ledgers.entries
                            .iter()
                            .filter(|(_, e)| {
                                matches!(e.state, InboundState::InProgress)
                                    && now.duration_since(e.last_touched) > ACQUIRE_TIMEOUT_INTERVAL * ACQUIRE_TIMEOUT_RETRIES_MAX
                                    // Only sweep if the acquisition never got meaningful data.
                                    // An acquisition with data is making progress in the tail —
                                    // killing it wastes all downloaded nodes.
                                    && e.last_touched.elapsed() > std::time::Duration::from_secs(60)
                            })
                            .map(|(k, _)| *k)
                            .collect();
                        for hash in stale {
                            if let Some(entry) = inbound_ledgers.entries.remove(&hash) {
                                let _ = entry.tx.send(AcqMsg::Stop);
                                tracing::info!(target: "bootstrap", seq = entry.seq, "Acquisition timed out (no progress for >60s)");
                            }
                        }

                        // Re-acquire latest validated target when slot is free
                        if let Some((target_hash, target_seq)) = last_validated_target
                            && target_seq > 1
                                && inbound_ledgers.active_count() == 0
                                && !inbound_ledgers.contains(&target_hash)
                            {
                                tracing::info!(target: "bootstrap", seq = target_seq, hash = %debug_hash8(&target_hash), "Acquiring validated target");
                                inbound_ledgers.acquire(target_hash, target_seq, validated);
                                inbound_ledgers.send_peers(&peers);
                            }
                    }

                    // --- History planner: drive fetch-pack requests when acquisitions stall ---
                    // issues fetch-pack requests to bulk-download history.
                    // Run even during initial sync (validated==0) using target_seq
                    // as the reference. reference doesn't gate this on validated state.
                    if target_seq > 1 && inbound_ledgers.active_count() > 0 {
                        use ledger::history_sync::*;

                        if let Some(lm) = app.ledger_master_runtime() {
                            history_sync_state.complete_ledgers =
                                lm.ledger_master().complete_ledgers();
                        }

                        struct LiveHashes<'a> {
                            inbound: &'a InboundLedgers,
                            hist_ledger: Option<std::sync::Arc<ledger::Ledger>>,
                            validated_ledger: Option<std::sync::Arc<ledger::Ledger>>,
                            loaded_runtime: Option<&'a app::AppLoadedLedgerRuntime>,
                        }
                        impl ledger::history_fetch::HistoryHashLookup for LiveHashes<'_> {
                            fn get_ledger_hash_for_history(
                                &self,
                                index: u32,
                                _reason: ledger::InboundLedgerReason,
                            ) -> Option<basics::sha_map_hash::SHAMapHash> {
                                hash_for_seq_from_available_sources(
                                    index,
                                    self.inbound,
                                    self.hist_ledger.as_ref(),
                                    self.validated_ledger.as_ref(),
                                    self.loaded_runtime,
                                )
                            }
                        }
                        struct LiveLedgerLookup<'a> {
                            loaded_runtime: Option<&'a app::AppLoadedLedgerRuntime>,
                        }
                        impl ledger::history_fetch::HistoryLedgerLookup<std::sync::Arc<ledger::Ledger>>
                            for LiveLedgerLookup<'_>
                        {
                            fn get_ledger_by_hash(
                                &self,
                                hash: basics::sha_map_hash::SHAMapHash,
                            ) -> Option<std::sync::Arc<ledger::Ledger>> {
                                self.loaded_runtime
                                    .and_then(|runtime| runtime.get_history_ledger_by_hash(hash).ok().flatten())
                            }
                        }
                        struct LiveInbound<'a> {
                            inbound: &'a InboundLedgers,
                            pending: &'a std::sync::Mutex<
                                Vec<(
                                    basics::sha_map_hash::SHAMapHash,
                                    u32,
                                    ledger::InboundLedgerReason,
                                )>,
                            >,
                            loaded_runtime: Option<&'a app::AppLoadedLedgerRuntime>,
                        }
                        impl
                            ledger::history_fetch::HistoryInboundAcquire<
                                std::sync::Arc<ledger::Ledger>,
                            > for LiveInbound<'_>
                        {
                            fn is_failure(&self, _: basics::sha_map_hash::SHAMapHash) -> bool {
                                false
                            }

                            fn acquire(
                                &self,
                                hash: basics::sha_map_hash::SHAMapHash,
                                seq: u32,
                                reason: ledger::InboundLedgerReason,
                            ) -> Option<std::sync::Arc<ledger::Ledger>> {
                                if let Some(ledger) = self
                                    .loaded_runtime
                                    .and_then(|runtime| runtime.get_history_ledger_by_hash(hash).ok().flatten())
                                {
                                    return Some(ledger);
                                }

                                if !self
                                    .inbound
                                    .entries
                                    .values()
                                    .any(|e| e.seq == seq || self.inbound.contains(hash.as_uint256()))
                                {
                                    let mut pending = self.pending.lock().expect("history pending acquires");
                                    if !pending.iter().any(|(pending_hash, pending_seq, _)| {
                                        *pending_seq == seq || *pending_hash == hash
                                    }) {
                                        pending.push((hash, seq, reason));
                                    }
                                }

                                None
                            }
                        }
                        struct LiveSql<'a> {
                            loaded_runtime: Option<&'a app::AppLoadedLedgerRuntime>,
                        }
                        impl ledger::history_fetch::HistorySqlInfo for LiveSql<'_> {
                            fn earliest_ledger_seq(&self) -> u32 {
                                self.loaded_runtime
                                    .map_or(1, |runtime| runtime.earliest_ledger_seq())
                            }

                            fn get_hash_by_index(
                                &self,
                                ledger_index: u32,
                            ) -> basics::sha_map_hash::SHAMapHash {
                                self.loaded_runtime
                                    .and_then(|runtime| runtime.get_hash_by_index(ledger_index))
                                    .unwrap_or_default()
                            }
                        }

                        // Build peer list for fetch-pack peer selection
                        struct PeerAdapter(Arc<dyn overlay::Peer>);
                        impl HistoryFetchPeer for PeerAdapter {
                            fn has_range(&self, min: u32, max: u32) -> bool { self.0.has_range(min, max) }
                            fn score(&self, clustered: bool) -> i32 { self.0.score(clustered) }
                        }
                        let adapted_peers: Vec<PeerAdapter> = peers.iter().map(|p| PeerAdapter(Arc::clone(p))).collect();

                        let now = time::Duration::seconds(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs() as i64,
                        );
                        let sync_config = ledger::LedgerHistorySyncConfig::new(256, 8, time::Duration::seconds(1));

                        let pending_history_acquires = std::sync::Mutex::new(Vec::<(
                            basics::sha_map_hash::SHAMapHash,
                            u32,
                            ledger::InboundLedgerReason,
                        )>::new());
                        let live_hashes = LiveHashes {
                            inbound: &inbound_ledgers,
                            hist_ledger: history_sync_state.fetch_state.hist_ledger.clone(),
                            validated_ledger: app.validated_ledger(),
                            loaded_runtime: loaded_ledger_runtime.as_ref(),
                        };
                        let live_lookup = LiveLedgerLookup {
                            loaded_runtime: loaded_ledger_runtime.as_ref(),
                        };
                        let live_inbound = LiveInbound {
                            inbound: &inbound_ledgers,
                            pending: &pending_history_acquires,
                            loaded_runtime: loaded_ledger_runtime.as_ref(),
                        };
                        let live_sql = LiveSql {
                            loaded_runtime: loaded_ledger_runtime.as_ref(),
                        };
                        let plan = run_history_advance(
                            if validated > 1 { validated } else { target_seq },
                            if validated > 1 { validated } else { target_seq },
                            None,
                            ledger::InboundLedgerReason::History,
                            now,
                            sync_config,
                            &history_sync_state,
                            &live_hashes,
                            &live_lookup,
                            &live_inbound,
                            &live_sql,
                            &adapted_peers,
                            false,
                        );

                        let mut next_history_sync_state = plan.next_state;
                        let mut start_planned_acq = |seq: u32, hash: Uint256| {
                            if seq <= 1 { return; }
                            if !bootstrap_acquire_budget_available(
                                validated,
                                inbound_ledgers.active_count(),
                                catchup_profile.ledger_fetch_limit,
                                inbound_ledgers.contains(&hash),
                            ) {
                                return;
                            }
                            inbound_ledgers.acquire(hash, seq, validated);
                        };

                        if let Some(clear_seq) = plan.clear_ledger
                            && let Some(lm) = app.ledger_master_runtime()
                        {
                            lm.ledger_master().clear_ledger(clear_seq);
                        }

                        if let Some(ledger) = plan.set_full_ledger.as_ref()
                            && let Some(lm) = app.ledger_master_runtime()
                        {
                            let persistence = ledger::LedgerPersistence::new(std::sync::Arc::new(
                                app.build_ledger_persistence_runtime(),
                            ));
                            let _ = lm.ledger_master().set_full_ledger(
                                &persistence,
                                std::sync::Arc::clone(ledger),
                                false,
                                false,
                                None,
                                None,
                            );
                        }

                        if let Some(ledger) = plan.schedule_try_fill.as_ref()
                            && let Some(lm) = app.ledger_master_runtime()
                            && let Some(runtime) = loaded_ledger_runtime.as_ref()
                        {
                            let ledger_master = lm.ledger_master();

                            struct LivePresence<'a> {
                                ledger_master: &'a app::AppLedgerMaster,
                            }
                            impl ledger::LedgerPresence for LivePresence<'_> {
                                fn have_ledger(&self, ledger_index: u32) -> bool {
                                    self.ledger_master.have_ledger(ledger_index)
                                }
                            }

                            struct LiveHashPairs<'a> {
                                runtime: &'a app::AppLoadedLedgerRuntime,
                            }
                            impl ledger::LedgerHashPairProvider for LiveHashPairs<'_> {
                                fn get_hashes_by_index(
                                    &self,
                                    min_seq: u32,
                                    max_seq: u32,
                                ) -> Vec<(u32, ledger::LedgerHashPair)> {
                                    self.runtime.get_hash_pairs_by_index(min_seq, max_seq)
                                }
                            }

                            struct LiveNodeStore<'a> {
                                runtime: &'a app::AppLoadedLedgerRuntime,
                            }
                            impl ledger::LedgerObjectPresence for LiveNodeStore<'_> {
                                fn has_ledger_object(
                                    &self,
                                    ledger_hash: basics::sha_map_hash::SHAMapHash,
                                    ledger_seq: u32,
                                ) -> bool {
                                    self.runtime.has_ledger_object(ledger_hash, ledger_seq)
                                }
                            }

                            struct LiveStopper<'a> {
                                app: &'a app::ApplicationRoot,
                            }
                            impl ledger::Stopper for LiveStopper<'_> {
                                fn is_stopping(&self) -> bool {
                                    self.app.is_stopping()
                                }
                            }

                            let ledger_header = ledger.header();
                            let fill_plan = ledger::fix_gaps(
                                &mut next_history_sync_state,
                                &ledger_header,
                                &LivePresence {
                                    ledger_master: ledger_master.as_ref(),
                                },
                                &LiveHashPairs { runtime },
                                &LiveNodeStore { runtime },
                                &LiveStopper { app: &app },
                            );
                            for range in &fill_plan.inserted_ranges {
                                for seq in range.min..=range.max {
                                    ledger_master.mark_ledger_complete(seq);
                                }
                            }
                        }

                        {
                            let pending = pending_history_acquires
                                .lock()
                                .expect("history pending acquires");
                            for (hash, seq, _) in pending.iter() {
                                start_planned_acq(*seq, *hash.as_uint256());
                            }
                        }

                        // Apply the plan: send fetch-pack request if one was generated
                        if let Some(fp) = &plan.fetch_pack
                            && let Some(peer) = adapted_peers.get(fp.peer_index) {
                                let wire = overlay::Message::new(fp.message.clone(), None);
                                peer.0.send(wire);
                                tracing::debug!(target: "inbound_ledger", peer_id = peer.0.id(), missing = fp.missing, "Fetch-pack request sent");
                            }

                        // Start prefetch acquisitions
                        for (seq, hash, _reason) in &plan.prefetch {
                            start_planned_acq(*seq, *hash.as_uint256());
                        }

                        history_sync_state = next_history_sync_state;
                    }


                    thread::sleep(Duration::from_millis(1));
                }
            })
            .expect("ledger catch-up thread should spawn");

        let mut guard = self
            .catch_up_state
            .worker
            .lock()
            .expect("ledger catch-up worker mutex poisoned");
        if let Some(previous) = guard.replace(handle) {
            let _ = previous.join();
        }
    }

    fn stop_catch_up_loop(&self) {
        tracing::info!(target: "main", "Shutdown signal received");
        self.catch_up_state.stop.store(true, Ordering::Release);
        if let Some(handle) = self
            .catch_up_state
            .worker
            .lock()
            .expect("ledger catch-up worker mutex poisoned")
            .take()
        {
            let _ = handle.join();
        }
    }
}

#[derive(Default)]
struct CatchUpState {
    stop: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
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
