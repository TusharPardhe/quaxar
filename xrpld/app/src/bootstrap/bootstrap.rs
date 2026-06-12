//! App-owned bootstrap assembly for the migrated runtime shell.
//!
//! This stays inside the app crate and only assembles the pieces that the app
//! crate can truthfully own today: config loading, `ApplicationRoot` setup,
//! default node-family ownership, optional SHAMap store ownership, and the
//! `MainRuntime` shell.

use crate::{
    ApplicationRoot, ApplicationRootOptions, BootstrapOverlayHandoff, DescriptorLimitProvider,
    LedgerReplay, MainRuntime, RclConsensusValidationSource, SHAMapStoreComponent,
    SHAMapStoreComponentRuntime, SHAMapStoreHealthRuntime, SHAMapStoreOperatingMode,
    SHAMapStoreRuntime, adjust_descriptor_limit, bootstrap_shamap_store,
};
use basics::base_uint::Uint256;
use basics::basic_config::{BasicConfig, IniFileSections};
use basics::string_utilities::str_unhex;
use basics::tagged_cache::MonotonicClock;
use ledger::{
    Ledger, LedgerConfig, LedgerHeader, LedgerInfoProvider, NullLedgerJournal,
    NullOrderBookDBJournal, NullOrderBookDBRuntime, load_by_hash, load_by_index,
};
use nodestore::{FetchType, ManagerImp, NodeObjectType as NodeStoreObjectType};
use protocol::{
    JsonValue, REGISTERED_FEATURES, RegisteredFeatureVote, STLedgerEntry, STParsedJSONObject,
    STTx, SerialIter, TxMeta, feature_id,
};
use rusqlite::{OptionalExtension, params};
use shamap::family::{
    NullFullBelowCache, NullMissingNodeReporter, SHAMapFamily, SHAMapNodeFetcher,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::node_object::NodeObject as SHAMapNodeObject;
use shamap::search::NodePathEntry;
use shamap::storage::NodeObjectType as SHAMapNodeObjectType;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;
use shamap::tree_node_cache::TreeNodeCache;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use xrpl_core::{ServiceRegistry, StartUpType};
use xrpld_core::{
    DatabaseCon, LEDGER_DB_INIT, LEDGER_DB_NAME, TRANSACTION_DB_INIT, TRANSACTION_DB_NAME,
    build_database_con_setup,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppBootstrapOptions {
    pub config_path: PathBuf,
    pub standalone: bool,
    pub start_valid: bool,
    pub elb_support: bool,
    pub io_threads: usize,
    pub job_queue_threads: usize,
    pub debug: bool,
    pub silent: bool,
    pub verbose: bool,
    pub quiet: bool,
    pub quorum: Option<usize>,
    pub newnodeid: bool,
    pub nodeid: Option<String>,
    pub definitions: bool,
    pub start_type: StartUpType,
    pub start_ledger: Option<String>,
    pub trap_tx_hash: Option<Uint256>,
    pub force_ledger_present_range: Option<(u32, u32)>,
    pub vacuum: bool,
    pub import: bool,
    pub rpc_ip: Option<String>,
    pub rpc_port: Option<u16>,
    pub unittest: Option<String>,
    pub unittest_arg: Option<String>,
    pub unittest_log: bool,
    pub unittest_ipv6: bool,
    pub unittest_jobs: Option<usize>,
    pub rpc_parameters: Vec<String>,
}

impl Default for AppBootstrapOptions {
    fn default() -> Self {
        Self {
            config_path: PathBuf::from("xrpld.cfg"),
            standalone: false,
            start_valid: false,
            elb_support: false,
            io_threads: 4,
            job_queue_threads: 1,
            debug: false,
            silent: false,
            verbose: false,
            quiet: false,
            quorum: None,
            newnodeid: false,
            nodeid: None,
            definitions: false,
            start_type: StartUpType::Fresh,
            start_ledger: None,
            trap_tx_hash: None,
            force_ledger_present_range: None,
            vacuum: false,
            import: false,
            rpc_ip: None,
            rpc_port: None,
            unittest: None,
            unittest_arg: None,
            unittest_log: false,
            unittest_ipv6: false,
            unittest_jobs: None,
            rpc_parameters: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppBootstrapReport {
    pub config_path: PathBuf,
    pub startup_ledger_mode: StartUpType,
    pub io_threads: usize,
    pub job_queue_threads: usize,
    pub ledger_history: u32,
    pub path_search_old: u32,
    pub path_search: u32,
    pub path_search_fast: u32,
    pub path_search_max: u32,
    pub has_overlay_runtime: bool,
    pub overlay_network_id: Option<u32>,
    pub cluster_node_count: usize,
    pub has_node_family: bool,
    pub has_server_ports_setup: bool,
    pub has_server_runtime: bool,
    pub server_configured_ports: Vec<String>,
    pub deferred_protocols: Vec<String>,
    pub has_resolver_runtime: bool,
    pub has_ledger_runtime: bool,
    pub has_ledger_master_runtime: bool,
    pub has_network_ops_runtime: bool,
    pub has_network_ops_validation_runtime: bool,
    pub has_consensus_runtime: bool,
    pub has_validator_site_runtime: bool,
    pub has_perf_log_runtime: bool,
    pub has_node_store: bool,
    pub node_store_kind: Option<String>,
    pub has_shamap_store_service: bool,
    pub fd_required: usize,
}

#[derive(Debug)]
pub struct AppBootstrapRoot {
    pub root: ApplicationRoot,
    pub report: AppBootstrapReport,
}

#[derive(Debug)]
pub struct AppBootstrapRuntime {
    pub runtime: Arc<MainRuntime>,
    pub report: AppBootstrapReport,
}

#[derive(Debug, Default)]
struct BootstrapSHAMapStoreRuntime {
    stopping: AtomicBool,
}

impl SHAMapStoreRuntime for BootstrapSHAMapStoreRuntime {
    fn start_background_work(&mut self) {}

    fn stop_background_work(&mut self) {
        self.stopping.store(true, Ordering::Release);
    }

    fn minimum_sql_seq(&self) -> Option<u32> {
        None
    }
}

impl SHAMapStoreHealthRuntime for BootstrapSHAMapStoreRuntime {
    fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }

    fn operating_mode(&self) -> SHAMapStoreOperatingMode {
        SHAMapStoreOperatingMode::Other
    }

    fn validated_ledger_age(&self) -> Duration {
        Duration::default()
    }
}

impl SHAMapStoreComponentRuntime for BootstrapSHAMapStoreRuntime {}

#[derive(Clone)]
struct BootstrapLedgerDbProvider {
    relational: Arc<crate::SqliteSHAMapStoreRelational>,
}

impl BootstrapLedgerDbProvider {
    fn new(relational: Arc<crate::SqliteSHAMapStoreRelational>) -> Self {
        Self { relational }
    }

    fn query_one(&self, sql: &str, bind: impl rusqlite::Params) -> Option<LedgerHeader> {
        let ledger_db = self.relational.ledger_db();
        let connection = ledger_db.get_session();
        connection
            .query_row(sql, bind, |row| {
                let close_time_resolution = row.get::<_, u32>(6)?;
                let close_flags = row.get::<_, u32>(7)?;
                Ok(LedgerHeader {
                    hash: parse_sql_hash(row.get::<_, String>(0)?)?,
                    seq: row.get::<_, u32>(1)?,
                    parent_hash: parse_sql_hash(row.get::<_, String>(2)?)?,
                    drops: row.get::<_, u64>(3)?,
                    close_time: row.get::<_, u32>(4)?,
                    parent_close_time: row.get::<_, u32>(5)?,
                    close_time_resolution: u8::try_from(close_time_resolution).map_err(|_| {
                        rusqlite::Error::FromSqlConversionFailure(
                            6,
                            rusqlite::types::Type::Integer,
                            Box::new(std::io::Error::other("invalid close time resolution")),
                        )
                    })?,
                    close_flags: u8::try_from(close_flags).map_err(|_| {
                        rusqlite::Error::FromSqlConversionFailure(
                            7,
                            rusqlite::types::Type::Integer,
                            Box::new(std::io::Error::other("invalid close flags")),
                        )
                    })?,
                    account_hash: parse_sql_hash(row.get::<_, String>(8)?)?,
                    tx_hash: parse_sql_hash(row.get::<_, String>(9)?)?,
                    ..LedgerHeader::default()
                })
            })
            .optional()
            .ok()
            .flatten()
    }
}

impl LedgerInfoProvider for BootstrapLedgerDbProvider {
    fn get_ledger_info_by_index(&self, ledger_index: u32) -> Option<LedgerHeader> {
        self.query_one(
            "SELECT LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime, PrevClosingTime, CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash FROM Ledgers WHERE LedgerSeq = ?1 ORDER BY LedgerSeq DESC LIMIT 1",
            params![i64::from(ledger_index)],
        )
    }

    fn get_ledger_info_by_hash(
        &self,
        ledger_hash: basics::sha_map_hash::SHAMapHash,
    ) -> Option<LedgerHeader> {
        self.query_one(
            "SELECT LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime, PrevClosingTime, CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash FROM Ledgers WHERE LedgerHash = ?1 LIMIT 1",
            params![ledger_hash.as_uint256().to_string()],
        )
    }

    fn get_newest_ledger_info(&self) -> Option<LedgerHeader> {
        self.query_one(
            "SELECT LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime, PrevClosingTime, CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash FROM Ledgers ORDER BY LedgerSeq DESC LIMIT 1",
            [],
        )
    }
}

#[derive(Clone)]
struct BootstrapNodeStoreFetcher {
    node_store: crate::SHAMapStoreNodeStore,
}

/// Minimal `ValidatorSiteSink` that delegates to `ValidatorList::apply_lists`
/// for the bootstrap initial validator-list fetch.
struct BootstrapValidatorSiteSink(
    Arc<crate::ValidatorList<crate::validator::validator_list::SystemValidatorListClock>>,
);

impl crate::ValidatorSiteSink for BootstrapValidatorSiteSink {
    fn apply_lists(
        &mut self,
        manifest: &str,
        version: u32,
        blobs: &[crate::ValidatorBlobInfo],
        site_uri: String,
        hash: basics::base_uint::Uint256,
    ) -> crate::PublisherListStats {
        self.0
            .apply_lists(manifest, version, blobs, site_uri, Some(hash))
    }

    fn load_lists(&self) -> Vec<String> {
        self.0.load_lists()
    }
}

impl BootstrapNodeStoreFetcher {
    fn new(node_store: crate::SHAMapStoreNodeStore) -> Self {
        Self { node_store }
    }
}

impl SHAMapNodeFetcher for BootstrapNodeStoreFetcher {
    fn fetch_node_object(
        &mut self,
        hash: basics::sha_map_hash::SHAMapHash,
        ledger_seq: u32,
    ) -> Option<SHAMapNodeObject> {
        let fetched = match &self.node_store {
            crate::SHAMapStoreNodeStore::Single(database) => database.fetch_node_object(
                hash.as_uint256(),
                ledger_seq,
                FetchType::Synchronous,
                false,
            ),
            crate::SHAMapStoreNodeStore::Rotating(database) => database.fetch_node_object(
                hash.as_uint256(),
                ledger_seq,
                FetchType::Synchronous,
                false,
            ),
        }?;

        Some(SHAMapNodeObject::new(
            match fetched.object_type() {
                NodeStoreObjectType::Ledger => SHAMapNodeObjectType::Ledger,
                NodeStoreObjectType::AccountNode => SHAMapNodeObjectType::AccountNode,
                NodeStoreObjectType::TransactionNode => SHAMapNodeObjectType::TransactionNode,
                NodeStoreObjectType::Unknown | NodeStoreObjectType::Dummy => {
                    SHAMapNodeObjectType::Unknown
                }
            },
            fetched.data().to_vec(),
            *fetched.hash(),
        ))
    }
}

pub fn parse_bootstrap_args<I>(args: I) -> Result<AppBootstrapOptions, String>
where
    I: IntoIterator<Item = String>,
{
    let mut options = AppBootstrapOptions::default();
    let mut iter = args.into_iter();
    let _ = iter.next(); // Skip binary name

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--conf" => {
                let Some(raw_path) = iter.next() else {
                    return Err("--conf requires a file path".to_owned());
                };
                options.config_path = PathBuf::from(raw_path);
            }
            "--debug" => {
                options.debug = true;
            }
            "--help" | "-h" => {
                return Err(usage());
            }
            "--quorum" => {
                let Some(raw_value) = iter.next() else {
                    return Err("--quorum requires a numeric value".to_owned());
                };
                options.quorum = Some(
                    raw_value
                        .parse::<usize>()
                        .map_err(|_| format!("invalid --quorum value: {raw_value}"))?,
                );
            }
            "--silent" => {
                options.silent = true;
            }
            "--standalone" | "-a" => {
                options.standalone = true;
            }
            "--verbose" | "-v" => {
                options.verbose = true;
            }
            "--quiet" | "-q" => {
                options.quiet = true;
            }
            "--newnodeid" => {
                options.newnodeid = true;
            }
            "--nodeid" => {
                let Some(id) = iter.next() else {
                    return Err("--nodeid requires a value".to_owned());
                };
                options.nodeid = Some(id);
            }
            "--definitions" => {
                options.definitions = true;
            }
            "--force_ledger_present_range" => {
                let Some(range_str) = iter.next() else {
                    return Err(
                        "--force_ledger_present_range requires a value (min,max)".to_owned()
                    );
                };
                let parts: Vec<&str> = range_str.split(',').collect();
                if parts.len() != 2 {
                    return Err(format!(
                        "invalid --force_ledger_present_range: expected min,max got {range_str}"
                    ));
                }
                let min = parts[0].parse::<u32>().map_err(|_| {
                    format!("invalid min in --force_ledger_present_range: {}", parts[0])
                })?;
                let max = parts[1].parse::<u32>().map_err(|_| {
                    format!("invalid max in --force_ledger_present_range: {}", parts[1])
                })?;
                options.force_ledger_present_range = Some((min, max));
            }
            "--version" => {
                options.rpc_parameters.push("version".to_string());
                return Ok(options);
            }
            "--import" => {
                options.import = true;
            }
            "--ledger" => {
                let Some(ledger) = iter.next() else {
                    return Err("--ledger requires a value".to_owned());
                };
                options.start_ledger = Some(ledger);
                if options.start_type != StartUpType::Replay {
                    options.start_type = StartUpType::Load;
                }
            }
            "--ledgerfile" => {
                let Some(ledger) = iter.next() else {
                    return Err("--ledgerfile requires a value".to_owned());
                };
                options.start_ledger = Some(ledger);
                options.start_type = StartUpType::LoadFile;
            }
            "--load" => {
                options.start_type = StartUpType::Load;
            }
            "--net" => {
                options.start_type = StartUpType::Network;
            }
            "--replay" => {
                options.start_type = StartUpType::Replay;
            }
            "--trap_tx_hash" => {
                let Some(hash_str) = iter.next() else {
                    return Err("--trap_tx_hash requires a hex value".to_owned());
                };
                let hash = Uint256::from_hex(&hash_str)
                    .map_err(|_| format!("invalid --trap_tx_hash value: {hash_str}"))?;
                options.trap_tx_hash = Some(hash);
            }
            "--start" => {
                options.start_type = StartUpType::Fresh;
                options.start_valid = true;
            }
            "--vacuum" => {
                options.vacuum = true;
            }
            "--valid" => {
                options.start_valid = true;
            }
            "--rpc" => {
                // Marker flag
            }
            "--rpc_ip" => {
                let Some(ip) = iter.next() else {
                    return Err("--rpc_ip requires a value".to_owned());
                };
                options.rpc_ip = Some(ip);
            }
            "--rpc_port" => {
                let Some(raw_port) = iter.next() else {
                    return Err("--rpc_port requires a numeric value".to_owned());
                };
                options.rpc_port = Some(
                    raw_port
                        .parse::<u16>()
                        .map_err(|_| format!("invalid --rpc_port value: {raw_port}"))?,
                );
            }
            "--unittest" | "-u" => {
                options.unittest = Some(iter.next().unwrap_or_default());
            }
            "--unittest-arg" => {
                options.unittest_arg = Some(iter.next().unwrap_or_default());
            }
            "--unittest-log" => {
                options.unittest_log = true;
            }
            "--unittest-ipv6" => {
                options.unittest_ipv6 = true;
            }
            "--unittest-jobs" => {
                let Some(raw_value) = iter.next() else {
                    return Err("--unittest-jobs requires a numeric value".to_owned());
                };
                options.unittest_jobs = Some(
                    raw_value
                        .parse::<usize>()
                        .map_err(|_| format!("invalid --unittest-jobs value: {raw_value}"))?,
                );
            }
            "--io-threads" => {
                let Some(raw_value) = iter.next() else {
                    return Err("--io-threads requires a numeric value".to_owned());
                };
                options.io_threads = raw_value
                    .parse::<usize>()
                    .map_err(|_| format!("invalid --io-threads value: {raw_value}"))?;
            }
            "--job-queue-threads" => {
                let Some(raw_value) = iter.next() else {
                    return Err("--job-queue-threads requires a numeric value".to_owned());
                };
                options.job_queue_threads = raw_value
                    .parse::<usize>()
                    .map_err(|_| format!("invalid --job-queue-threads value: {raw_value}"))?;
            }
            other if other.starts_with('-') => {
                return Err(format!("unrecognized argument: {other}"));
            }
            positional => {
                options.rpc_parameters.push(positional.to_string());
            }
        }
    }

    Ok(options)
}

pub fn load_basic_config_file(path: impl AsRef<Path>) -> Result<BasicConfig, String> {
    let path = path.as_ref();
    tracing::info!(target: "bootstrap", config_path = %path.display(), "Loading configuration");
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read config file {}: {error}", path.display()))?;
    let mut config = parse_basic_config_text(&contents)?;

    // Load [validators_file] if present (mimics C++ Config::loadValidatorFile)
    if config.exists("validators_file") {
        let validators_file = config
            .section("validators_file")
            .legacy()
            .unwrap_or_default();
        if !validators_file.is_empty() {
            let vf_path = if Path::new(&validators_file).is_absolute() {
                PathBuf::from(&validators_file)
            } else {
                path.parent()
                    .unwrap_or(Path::new("."))
                    .join(&validators_file)
            };
            match fs::read_to_string(&vf_path) {
                Ok(vf_contents) => {
                    tracing::info!(target: "bootstrap", path = %vf_path.display(), "Loading validators file");
                    let vf_config = parse_basic_config_text(&vf_contents)?;
                    // Merge validator sections into main config
                    for section_name in
                        ["validator_list_sites", "validator_list_keys", "validators"]
                    {
                        if vf_config.exists(section_name) {
                            let values = vf_config.section(section_name).values().to_vec();
                            if !values.is_empty() {
                                config.section_mut(section_name).append_lines(values);
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(target: "bootstrap", path = %vf_path.display(), error = %e, "Failed to load validators file");
                }
            }
        }
    }

    Ok(config)
}

pub fn build_bootstrap_runtime(
    config: &BasicConfig,
    options: &AppBootstrapOptions,
) -> Result<AppBootstrapRuntime, String> {
    let bootstrap = build_bootstrap_root(config, options)?;
    let runtime = Arc::new(MainRuntime::new(bootstrap.root));
    // - start_valid → Full (node starts fully synced)
    // - non-standalone → Connected (node starts connected to network)
    if !options.standalone {
        use crate::network::network_ops::NetworkOpsOperatingMode;
        let mode = if options.start_valid {
            NetworkOpsOperatingMode::Full
        } else {
            NetworkOpsOperatingMode::Connected
        };
        runtime.root().set_network_ops_operating_mode(mode);
    }
    Ok(AppBootstrapRuntime {
        runtime,
        report: bootstrap.report,
    })
}

pub fn build_bootstrap_root(
    config: &BasicConfig,
    options: &AppBootstrapOptions,
) -> Result<AppBootstrapRoot, String> {
    let io_threads = config_legacy_usize(config, "io_workers").unwrap_or(options.io_threads);
    let job_queue_threads = config_legacy_usize(config, "workers")
        .unwrap_or(options.job_queue_threads)
        .max(1);
    let ledger_history = config_legacy_u32(config, "ledger_history").unwrap_or(0);
    let path_search_old = config_legacy_u32(config, "path_search_old").unwrap_or(2);
    let path_search = config_legacy_u32(config, "path_search").unwrap_or(2);
    let path_search_fast = config_legacy_u32(config, "path_search_fast").unwrap_or(2);
    let path_search_max = config_path_search_max(config);

    let mut root = ApplicationRoot::with_options(ApplicationRootOptions {
        io_threads,
        job_queue_threads,
        start_valid: options.start_valid,
        elb_support: options.elb_support,
        standalone: options.standalone,
        start_type: options.start_type,
        start_ledger: options.start_ledger.clone(),
        import: options.import,
        quorum: options.quorum,
        ..ApplicationRootOptions::default()
    })
    .map_err(|error| error.to_string())?;

    root.set_path_search_levels(path_search_old, path_search, path_search_fast);
    let _ = root.set_path_search_max(path_search_max);
    let _ = root.attach_default_resolver_runtime();
    let _ = root.attach_default_ledger_master_runtime();
    let _ = root.attach_default_network_ops_validation_runtime();
    let _ = root.attach_default_network_ops_runtime();
    attach_relational_database_if_configured(&mut root, config, options, ledger_history)?;
    let _ = root
        .attach_server_ports_from_config(config, options.standalone)
        .map_err(|error| error.to_string())?;
    let _ = root.load_peer_reservations()?;
    let _ = root.load_cluster_nodes_from_config(config)?;
    let _ = root.attach_configured_overlay_runtime(config, Arc::new(BootstrapOverlayHandoff))?;

    // Load validation seed into config BEFORE consensus runtime is created,
    // so the consensus adaptor can read it.
    if let Ok(seed) = config.legacy("validation_seed") {
        root.set_validation_seed(seed);
    }

    let _ = root.attach_default_consensus_runtime();
    let node_store_kind = attach_shamap_store_if_configured(
        &mut root,
        config,
        options.standalone,
        ledger_history,
        io_threads,
    )?;
    let configured_node_size = configured_node_size_from_config(config);
    root.set_status_rpc_node_size(configured_node_size.clone());
    attach_bootstrap_node_family(&mut root, configured_node_size.as_deref());
    initialize_startup_ledger_state(&root, options, config)?;
    root.bind_default_component_runtimes();

    // Wire up node identity (pubkey_node in server_info) from wallet DB,
    // matching reference Application::setup() -> getNodeIdentity().
    {
        use crate::state::node_identity::load_or_generate_node_identity;
        let identity = load_or_generate_node_identity(&root.wallet_db());
        root.set_node_identity(identity);
    }

    // Wire up validator list publisher keys from config, matching reference
    // Application::setup() → validators->load(...)
    {
        let publisher_keys: Vec<String> = config.section("validator_list_keys").values().to_vec();
        tracing::debug!(target: "bootstrap", ?publisher_keys, "Validator list keys loaded");
        let config_keys: Vec<String> = config.section("validators").values().to_vec();
        let _ = root.validators().load(
            root.validation_public_key(),
            &config_keys,
            &publisher_keys,
            None,
        );
        // When using static [validators] (no validator_list_sites), we must
        // explicitly promote key_listings to trusted_master_keys. Without this,
        // validations from peers are dropped as "untrusted".
        if config.section("validator_list_sites").empty() && !config_keys.is_empty() {
            root.validators().update_trusted(&std::collections::HashSet::new(), 0);
        }
    }

    // Wire up validator list sites from config and do initial fetch,
    // matching reference Application::setup() → validatorSites_->start()
    {
        let site_uris: Vec<String> = config.section("validator_list_sites").values().to_vec();
        tracing::debug!(target: "bootstrap", ?site_uris, "Validator list sites loaded");
        if !site_uris.is_empty() {
            let validators = root.validators();
            let mut site = crate::ValidatorSite::new(std::time::Duration::from_secs(30));
            site.load(&site_uris);
            let mut sink = BootstrapValidatorSiteSink(validators.clone());
            let transport = crate::ReqwestValidatorSiteTransport;
            site.refresh_due(&mut sink, &transport, std::time::SystemTime::now());
            // Mark validators as trusted after loading the list.
            validators.update_trusted(&std::collections::HashSet::new(), 0);
        }
    }

    let report = AppBootstrapReport {
        config_path: options.config_path.clone(),
        startup_ledger_mode: options.start_type,
        io_threads,
        job_queue_threads,
        ledger_history,
        path_search_old,
        path_search,
        path_search_fast,
        path_search_max,
        has_overlay_runtime: root.overlay_runtime().is_some(),
        overlay_network_id: root
            .overlay_runtime()
            .and_then(|overlay| overlay.network_id()),
        cluster_node_count: root.shared_cluster().size(),
        has_node_family: root.node_family().is_some(),
        has_server_ports_setup: root.server_ports_setup().is_some(),
        has_server_runtime: root.runtime_bindings().server.is_some(),
        server_configured_ports: root
            .server_ports_setup()
            .map(|setup| setup.ports.iter().map(|port| port.name.clone()).collect())
            .unwrap_or_default(),
        deferred_protocols: root.server_handler().snapshot().deferred_protocols,
        has_resolver_runtime: root.resolver_runtime().is_some(),
        has_ledger_runtime: root.runtime_bindings().ledger.is_some(),
        has_ledger_master_runtime: root.ledger_master_runtime().is_some(),
        has_network_ops_runtime: root.network_ops_runtime().is_some(),
        has_network_ops_validation_runtime: root.network_ops_validation_runtime().is_some(),
        has_consensus_runtime: root.consensus_runtime().is_some(),
        has_validator_site_runtime: root.runtime_bindings().validator_site.is_some(),
        has_perf_log_runtime: root.runtime_bindings().perf_log.is_some(),
        has_node_store: node_store_kind.is_some(),
        node_store_kind,
        has_shamap_store_service: root.shamap_store_service().is_some(),
        fd_required: root.fd_required(),
    };

    Ok(AppBootstrapRoot { root, report })
}

pub fn build_bootstrap_runtime_from_path(
    path: impl AsRef<Path>,
    mut options: AppBootstrapOptions,
) -> Result<AppBootstrapRuntime, String> {
    options.config_path = path.as_ref().to_path_buf();
    let config = load_basic_config_file(&options.config_path)?;
    build_bootstrap_runtime(&config, &options)
}

pub fn build_bootstrap_runtime_from_args<I>(args: I) -> Result<AppBootstrapRuntime, String>
where
    I: IntoIterator<Item = String>,
{
    let options = parse_bootstrap_args(args)?;
    let config_path = options.config_path.clone();
    build_bootstrap_runtime_from_path(config_path, options)
}

pub fn run_from_args<I>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = String>,
{
    let bootstrap = build_bootstrap_runtime_from_args(args)?;
    run_bootstrap_runtime(bootstrap)
}

pub fn run_bootstrap_runtime(bootstrap: AppBootstrapRuntime) -> Result<(), String> {
    let runtime = Arc::clone(&bootstrap.runtime);
    ensure_descriptor_budget(bootstrap.report.fd_required)?;
    runtime.start()?;
    tracing::info!(target: "app", "Node startup complete");

    // For --start mode: store genesis as closed ledger so consensus can find
    // it as a parent. The first round is started in the event loop once peers
    // are connected (so proposals arrive before the idle timeout closes it).
    if bootstrap.report.startup_ledger_mode == StartUpType::Fresh {
        let root = runtime.root();
        if let Some(lm) = root.ledger_master_runtime() {
            if let Some(validated) = root.validated_ledger() {
                lm.ledger_master().set_closed_ledger(Arc::clone(&validated));
            }
        }
    }

    // Spawn a dedicated consensus event loop for --start mode (private networks).
    //
    // WHY: In normal operation, the catchup loop in main.rs drives consensus by
    // draining proposals/validations from the overlay and ticking the consensus
    // timer. However, when using --start mode (StartUpType::Fresh), the node
    // boots directly into bootstrap without entering the catchup loop. Without
    // this thread, proposals and validations from peers are never consumed and
    // the consensus timer never fires, so the network stalls after genesis.
    //
    // This thread replicates only the consensus-driving subset of the catchup
    // loop: proposal processing, validation processing, map-complete handling,
    // and timer ticks. It does NOT do ledger acquisition or inbound ledger
    // processing — those are unnecessary when starting fresh.
    let consensus_stop = Arc::new(AtomicBool::new(false));
    let consensus_thread = if matches!(
        bootstrap.report.startup_ledger_mode,
        StartUpType::Fresh | StartUpType::Network
    ) && bootstrap.report.has_overlay_runtime
    {
        // This thread exclusively drives consensus in --start mode.
        // Set need_network_ledger to prevent the main validation processor
        // thread from also driving proposals/timer (which causes divergence).
        runtime.root().set_need_network_ledger(true);
        let stop_flag = Arc::clone(&consensus_stop);
        let rt = Arc::clone(&runtime);
        Some(std::thread::Builder::new()
            .name("start-mode-consensus".into())
            .spawn(move || {
                run_start_mode_consensus_loop(&rt, &stop_flag);
            })
            .expect("failed to spawn start-mode-consensus thread"))
    } else {
        None
    };

    let stop_requested = Arc::new(AtomicBool::new(false));
    let stop_thread = spawn_shutdown_watcher(Arc::clone(&runtime), Arc::clone(&stop_requested));

    runtime.run();

    // Signal the consensus event loop to stop, then join it.
    consensus_stop.store(true, Ordering::Release);
    if let Some(handle) = consensus_thread {
        let _ = handle.join();
    }

    stop_requested.store(true, Ordering::Release);
    let _ = stop_thread.join();
    Ok(())
}

/// Consensus event loop for --start mode private networks.
///
/// Drains proposals and validations from the overlay, processes map-complete
/// results, and ticks the consensus timer on a ~200ms cadence. This drives
/// the consensus state machine that would otherwise only run inside the
/// catchup loop (which is never entered in --start mode).
fn run_start_mode_consensus_loop(runtime: &MainRuntime, stop: &AtomicBool) {
    use consensus;

    tracing::info!(target: "consensus", "Start-mode consensus event loop running");

    // Take the map-complete receiver once (it's a take-once resource).
    let map_complete_rx = runtime
        .root()
        .consensus_runtime()
        .and_then(|cr| cr.take_map_complete_receiver());

    let mut consensus_started = false;

    // State for consensus ledger acquisition with targeted peer requests.

    while !stop.load(Ordering::Acquire) {
        let root = runtime.root();

        let (Some(overlay_rt), Some(consensus_rt), Some(network_ops_rt)) = (
            root.overlay_runtime(),
            root.consensus_runtime(),
            root.network_ops_runtime(),
        ) else {
            std::thread::sleep(Duration::from_millis(500));
            continue;
        };

        // Serve TmGetLedger requests from peers (PART 1).
        serve_get_ledger_requests(root, &overlay_rt);

        // Broadcast our closed ledger to peers via StatusChange so they
        // can detect whether we're at genesis or ahead. This replaces the
        // overlay timer StatusChange that doesn't run in --start mode.
        if !consensus_started {
            if let Some(lm_rt) = root.ledger_master_runtime() {
                if let Some(closed) = lm_rt.ledger_master().closed_ledger() {
                    use overlay::Overlay;
                    let hdr = closed.header();
                    let status = overlay::ProtocolMessage::new(
                        overlay::ProtocolPayload::StatusChange(
                            overlay::message::wire::TmStatusChange {
                                new_status: Some(1),
                                new_event: Some(1),
                                ledger_seq: Some(hdr.seq),
                                ledger_hash: Some(hdr.hash.as_uint256().data().to_vec()),
                                ledger_hash_previous: Some(hdr.parent_hash.as_uint256().data().to_vec()),
                                network_time: None,
                                first_seq: Some(1),
                                last_seq: Some(hdr.seq),
                            },
                        ),
                    );
                    overlay_rt.overlay().broadcast(&status);
                }
            }
        }

        // Before starting consensus, acquire the network's validated ledger.
        if !consensus_started {
            // Check peers for a ledger we need to acquire.
            if true {
                use overlay::Overlay;
                let peers = overlay_rt.overlay().active_peers();
                if !peers.is_empty() {
                    let any_ahead = peers.iter().find(|p| {
                        let h = p.closed_ledger_hash();
                        if h.is_zero() {
                            return false;
                        }
                        if let Some(lm_rt) = root.ledger_master_runtime() {
                            lm_rt.ledger_master().get_ledger_by_hash(
                                basics::sha_map_hash::SHAMapHash::new(h),
                            ).is_none()
                        } else {
                            false
                        }
                    });
                    if let Some(peer) = any_ahead {
                        // Trigger acquisition for that peer's ledger.
                        let closed_hash = peer.closed_ledger_hash();
                        if let Some(lm_rt) = root.ledger_master_runtime() {
                            let mut pending = lm_rt.pending_consensus_ledger
                                .lock()
                                .expect("pending_consensus_ledger lock");
                            if pending.is_none() {
                                *pending = Some(closed_hash);
                                tracing::info!(target: "consensus",
                                    hash = %closed_hash,
                                    peer_id = peer.id(),
                                    "Requesting peer's current closed ledger"
                                );
                            }
                        }
                    } else {
                        // All peers are at genesis or we already have their
                        // ledger. Only start if at least one peer has actively
                        // reported their closed ledger (non-zero). Peers that
                        // haven't sent StatusChange yet report zero — wait for
                        // them to confirm before starting.
                        if let Some(lm_rt) = root.ledger_master_runtime() {
                            if let Some(closed) = lm_rt.ledger_master().closed_ledger() {
                                let closed_hash = *closed.header().hash.as_uint256();
                                let any_confirmed = peers.iter().any(|p| {
                                    !p.closed_ledger_hash().is_zero()
                                });
                                if any_confirmed {
                                    // Drain queued proposals into the consensus
                                    // engine BEFORE starting so that startRound's
                                    // playback_proposals finds them (matching
                                    // rippled where proposals arrive via JobQueue
                                    // before startRound runs).
                                    let proposals = overlay_rt.overlay().take_proposals();
                                    for proposal in &proposals {
                                        let close_time = root.shared_time_keeper().close_time();
                                        let prop = consensus::ConsensusProposal::new(
                                            proposal.previous_ledger, 0,
                                            proposal.current_tx_hash,
                                            close_time, close_time,
                                            proposal.public_key,
                                        );
                                        let _ = network_ops_rt.handle_peer_proposal(
                                            consensus_rt.as_ref(),
                                            proposal.public_key,
                                            proposal.message.signature.clone(),
                                            proposal.suppression,
                                            prop,
                                        );
                                    }
                                    if network_ops_rt.maybe_begin_consensus_from_validated(
                                        consensus_rt.as_ref(),
                                        Arc::clone(&closed),
                                    ) {
                                        tracing::info!(target: "consensus",
                                            seq = closed.header().seq,
                                            "Consensus started — peers confirmed current ledger"
                                        );
                                        consensus_started = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Process validations to sync clock and detect network state.
            let validations = overlay_rt.overlay().take_validations();
            for queued in &validations {
                let mut serial = protocol::SerialIter::new(&queued.message.validation);
                let parsed =
                    protocol::STValidation::from_serial_iter_default_node_id(&mut serial, false);
                let mut validation = match parsed {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                // Adjust clock from validator sign_time FIRST so that
                // set_seen and is_current use a synchronized clock.
                let sign_time = validation.get_sign_time();
                if sign_time > 0 {
                    let now = root.current_close_time_seconds() as i64;
                    let offset = sign_time as i64 - now;
                    root.time_keeper()
                        .adjust_close_time(time::Duration::seconds(offset));
                }
                // Set seen_time AFTER clock adjustment so is_current passes.
                validation.set_seen(root.current_close_time_seconds());
                let source = queued.peer_id.to_string();
                let _ = root.receive_validation_to_network_ops(&mut validation, &source);

                // If this is a trusted validation for a ledger we don't have,
                // trigger acquisition from the peer that sent it.
                if validation.is_trusted() {
                    let val_hash = validation.get_ledger_hash();
                    let val_seq = validation.get_field_u32(
                        protocol::get_field_by_symbol("sfLedgerSequence"));
                    if let Some(lm_rt) = root.ledger_master_runtime() {
                        let lm = lm_rt.ledger_master();
                        if lm.get_ledger_by_hash(
                            basics::sha_map_hash::SHAMapHash::new(val_hash),
                        ).is_none() && val_seq > 1 {
                            let mut pending = lm_rt.pending_consensus_ledger
                                .lock()
                                .expect("pending_consensus_ledger lock");
                            if pending.is_none() {
                                *pending = Some(val_hash);
                                tracing::info!(target: "consensus",
                                    hash = %val_hash, seq = val_seq,
                                    peer_id = queued.peer_id,
                                    "Trusted validation for missing ledger — triggering acquisition"
                                );
                            }
                        }
                    }
                }
            }

            // If acquisition completed, closed_ledger updated → start consensus
            if !consensus_started {
                if let Some(lm_rt) = root.ledger_master_runtime() {
                    if let Some(closed) = lm_rt.ledger_master().closed_ledger() {
                        if closed.header().seq > 1 {
                            if network_ops_rt.maybe_begin_consensus_from_validated(
                                consensus_rt.as_ref(),
                                Arc::clone(&closed),
                            ) {
                                tracing::info!(target: "consensus",
                                    seq = closed.header().seq,
                                    "Consensus started from acquired network ledger"
                                );
                                consensus_started = true;
                            }
                        }
                    }
                }
            }

            std::thread::sleep(Duration::from_millis(50));
            continue;
        }

        // Process map-complete results (TX set acquisitions from peers).
        if let Some(ref rx) = map_complete_rx {
            while let Ok((hash, set)) = rx.try_recv() {
                network_ops_rt.handle_map_complete(consensus_rt.as_ref(), hash, set);
            }
        }

        // Drain and feed peer proposals into the consensus engine.
        if consensus_started {
        let proposals = overlay_rt.overlay().take_proposals();
        for proposal in &proposals {
            // If peers are proposing on a ledger we don't have, trigger
            // acquisition so we can switch to their chain.
            if let Some(lm_rt) = root.ledger_master_runtime() {
                let lm = lm_rt.ledger_master();
                if lm.get_ledger_by_hash(
                    basics::sha_map_hash::SHAMapHash::new(proposal.previous_ledger),
                ).is_none() {
                    let mut pending = lm_rt.pending_consensus_ledger
                        .lock()
                        .expect("pending_consensus_ledger lock");
                    if pending.is_none() {
                        *pending = Some(proposal.previous_ledger);
                        tracing::info!(target: "consensus",
                            hash = %proposal.previous_ledger,
                            "Triggering acquisition for peer's previous_ledger"
                        );
                    }
                }
            }
        }
        for proposal in proposals {
            let close_time = root.shared_time_keeper().close_time();
            let prop = consensus::ConsensusProposal::new(
                proposal.previous_ledger,
                0,
                proposal.current_tx_hash,
                close_time,
                close_time,
                proposal.public_key,
            );
            let _ = network_ops_rt.handle_peer_proposal(
                consensus_rt.as_ref(),
                proposal.public_key,
                proposal.message.signature.clone(),
                proposal.suppression,
                prop,
            );
        }
        }

        // Drain and process inbound validations.
        let validations = overlay_rt.overlay().take_validations();
        for queued in &validations {
            let mut serial = protocol::SerialIter::new(&queued.message.validation);
            let parsed =
                protocol::STValidation::from_serial_iter_default_node_id(&mut serial, false);
            let mut validation = match parsed {
                Ok(v) => v,
                Err(_) => continue,
            };
            let sign_time = validation.get_sign_time();
            if sign_time > 0 {
                let now = root.current_close_time_seconds() as i64;
                let offset = sign_time as i64 - now;
                root.time_keeper()
                    .adjust_close_time(time::Duration::seconds(offset));
            }
            validation.set_seen(root.current_close_time_seconds());
            let source = queued.peer_id.to_string();
            let _ = root.receive_validation_to_network_ops(&mut validation, &source);
        }

        // Tick the consensus state machine (rippled heartbeat equivalent).
        network_ops_rt.handle_consensus_timer(consensus_rt.as_ref());

        // Promote the closed ledger to validated once it reaches quorum.
        if let Some(lm_rt) = root.ledger_master_runtime() {
            let lm = lm_rt.ledger_master();
            let valid_seq = lm.valid_ledger_seq();
            let quorum = root.validators().quorum();
            if let Some(closed) = lm.closed_ledger() {
                let closed_seq = closed.header().seq;
                if closed_seq > valid_seq {
                    let closed_hash = *closed.header().hash.as_uint256();
                    let val_count = root.validations().num_trusted_for_ledger(closed_hash);
                    if val_count >= quorum {
                        let mut l = (*closed).clone();
                        l.set_validated();
                        let validated = std::sync::Arc::new(l);
                        lm.set_valid_ledger_no_sweep(
                            std::sync::Arc::clone(&validated),
                            None,
                            None,
                        );
                        root.note_validated_ledger_for_sync(
                            std::sync::Arc::clone(&validated),
                        );
                        root.set_need_network_ledger(false);
                        tracing::info!(target: "consensus",
                            seq = closed_seq, val_count, quorum,
                            "Validated ledger advanced (--start mode)"
                        );
                    }
                }
            }
        }

        // --- Consensus ledger acquisition ---
        std::thread::sleep(Duration::from_millis(50));
    }

    tracing::info!(target: "consensus", "Start-mode consensus event loop stopped");
}

/// State for targeted ledger acquisition with peer rotation and 2s retry.
fn serve_get_ledger_requests(
    root: &crate::ApplicationRoot,
    overlay_rt: &Arc<crate::runtime::overlay_runtime::AppOverlayRuntime>,
) {
    use overlay::Overlay;

    let requests = overlay_rt.overlay().take_get_ledgers();
    if requests.is_empty() {
        return;
    }
    let Some(lm_rt) = root.ledger_master_runtime() else {
        return;
    };
    let lm = lm_rt.ledger_master();

    for req in requests {
        // Only serve li_BASE (itype=0) requests by hash.
        if req.message.itype != 0 {
            continue;
        }
        let Some(hash_bytes) = req.message.ledger_hash.as_deref() else {
            continue;
        };
        let Some(hash) = Uint256::from_slice(hash_bytes) else {
            continue;
        };
        let Some(ledger) = lm.get_ledger_by_hash(
            basics::sha_map_hash::SHAMapHash::new(hash),
        ) else {
            continue;
        };

        // Serialize header and send TmLedgerData response.
        let header_data = protocol::serialize_prefixed_ledger_header(&ledger.header(), true);
        let response = overlay::ProtocolMessage::new(overlay::ProtocolPayload::LedgerData(
            overlay::TmLedgerData {
                ledger_hash: hash.data().to_vec(),
                ledger_seq: ledger.header().seq,
                r#type: 0, // li_BASE
                nodes: vec![overlay::message::wire::TmLedgerNode {
                    nodeid: None,
                    nodedata: header_data,
                }],
                request_cookie: req.message.request_cookie.map(|c| c as u32),
                error: None,
            },
        ));
        let message = overlay::Message::new(response, None);
        // Send to the requesting peer.
        if let Some(peer) = overlay_rt.overlay().find_peer_by_short_id(req.peer_id) {
            peer.send(message);
        }
    }
}







fn ensure_descriptor_budget(required: usize) -> Result<(), String> {
    let required = required.max(1024) as u64;
    let provider = SystemDescriptorLimitProvider;
    if adjust_descriptor_limit(required, &provider) {
        Ok(())
    } else {
        Err(format!(
            "Insufficient number of file descriptors: {required} are needed"
        ))
    }
}

fn spawn_shutdown_watcher(
    runtime: Arc<MainRuntime>,
    stop_requested: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        loop {
            if stop_requested.load(Ordering::Acquire) {
                return;
            }

            let ctrl_c_seen = runtime.root().basic_app().block_on(async {
                tokio::select! {
                    result = tokio::signal::ctrl_c() => result.is_ok(),
                    _ = tokio::time::sleep(Duration::from_millis(100)) => false,
                }
            });

            if stop_requested.load(Ordering::Acquire) {
                return;
            }

            if ctrl_c_seen {
                let _ = runtime.signal_stop("received shutdown signal");
                return;
            }
        }
    })
}

fn attach_shamap_store_if_configured(
    root: &mut ApplicationRoot,
    config: &BasicConfig,
    standalone: bool,
    ledger_history: u32,
    io_threads: usize,
) -> Result<Option<String>, String> {
    if !config.exists("node_db") {
        return Ok(None);
    }

    let manager = ManagerImp::new();
    let scheduler = Arc::new(root.node_store_scheduler().clone());
    let journal = root.get_journal("NodeStore");
    let bootstrap = bootstrap_shamap_store(
        config,
        standalone,
        ledger_history,
        io_threads.max(1) as i32,
        40_000,
        64,
        2,
        &manager,
        scheduler,
        journal,
    )?;
    let node_store_kind = bootstrap.node_store_kind().to_owned();
    let _ = bootstrap.attach_node_store(root);
    let component = Arc::new(SHAMapStoreComponent::new(
        bootstrap.store.clone(),
        Box::new(BootstrapSHAMapStoreRuntime::default()),
        bootstrap.state_db,
    ));
    let _ = root.attach_shamap_store_component(component);
    Ok(Some(node_store_kind))
}

fn attach_relational_database_if_configured(
    root: &mut ApplicationRoot,
    config: &BasicConfig,
    options: &AppBootstrapOptions,
    ledger_history: u32,
) -> Result<bool, String> {
    if !config.exists("database_path") {
        return Ok(false);
    }

    let setup = build_database_con_setup(
        config,
        to_xrpld_startup_type(options.start_type),
        options.standalone,
        ledger_history,
    )?;
    if !setup.data_dir.as_os_str().is_empty() {
        if let Err(error) = fs::create_dir_all(&setup.data_dir) {
            let is_existing_dir = setup.data_dir.is_dir();
            if !is_existing_dir {
                return Err(format!(
                    "failed to create bootstrap database directory {}: {error}",
                    setup.data_dir.display()
                ));
            }
        }
    }
    let ledger_db = Arc::new(DatabaseCon::new_from_setup(
        &setup,
        LEDGER_DB_NAME,
        &setup.lgr_pragma,
        LEDGER_DB_INIT,
    )?);
    let transaction_db = Arc::new(DatabaseCon::new_from_setup(
        &setup,
        TRANSACTION_DB_NAME,
        &setup.tx_pragma,
        TRANSACTION_DB_INIT,
    )?);
    let relational = Arc::new(crate::SqliteSHAMapStoreRelational::new(
        ledger_db,
        Some(transaction_db),
        true,
        100,
        Duration::from_millis(0),
    ));
    let _ = root.attach_relational_database(Some(relational));

    // Open rdb::LedgerDb for header persistence (compatibility: the reference source Ledgers table).
    // Used on restart to load the last validated ledger without peer re-acquisition.
    let rdb_path = setup.data_dir.join("ledger_headers.db");
    tracing::info!(target: "ledger",
        "[bootstrap] opening ledger_headers.db at {}",
        rdb_path.display()
    );
    match rdb::LedgerDb::open(&rdb_path) {
        Ok(db) => {
            root.attach_ledger_db(Some(std::sync::Arc::new(db)));
        }
        Err(e) => {
            tracing::info!(target: "ledger", "[bootstrap] failed to open ledger_headers.db: {e}");
        }
    }

    Ok(true)
}

fn configured_node_size_from_config(config: &BasicConfig) -> Option<String> {
    if !config.exists("node_size") {
        return None;
    }

    let section = config.section("node_size");
    match section.values() {
        [node_size] => {
            let node_size = node_size.trim().to_ascii_lowercase();
            match node_size.as_str() {
                "tiny" | "small" | "medium" | "large" | "huge" => Some(node_size),
                _ => None,
            }
        }
        values => {
            tracing::warn!(
                "Section 'node_size': requires 1 line not {} lines.",
                values.len()
            );
            None
        }
    }
}

fn attach_bootstrap_node_family(root: &mut ApplicationRoot, node_size: Option<&str>) {
    if let Some(node_store) = root.node_store().clone() {
        let profile = crate::NodeSizeResourceProfile::for_node_size(node_size);
        let family = crate::NodeFamily::new(SHAMapFamily::new(
            Arc::new(TreeNodeCache::new(
                "app-bootstrap-node-family",
                profile.tree_cache_size,
                time::Duration::seconds(profile.tree_cache_age_seconds),
                MonotonicClock::default(),
            )),
            NullFullBelowCache::new(0),
            BootstrapNodeStoreFetcher::new(node_store),
            NullMissingNodeReporter,
        ));
        let _ = root.attach_node_family(Arc::new(family));
        let _ = root.wire_node_family_reset();
        return;
    }

    let _ = root.attach_default_node_family();
}

fn initialize_startup_ledger_state(
    root: &ApplicationRoot,
    options: &AppBootstrapOptions,
    config: &BasicConfig,
) -> Result<(), String> {
    match options.start_type {
        StartUpType::Load => load_startup_ledger_from_storage(root, options),
        StartUpType::Replay => replay_startup_ledger_from_storage(root, options),
        StartUpType::LoadFile => load_startup_ledger_from_file(root, options),
        StartUpType::Network => {
            if !root.config().standalone {
                root.set_need_network_ledger(true);
            }
            seed_startup_ledger_state(root, options, config)
        }
        StartUpType::Fresh | StartUpType::Normal | StartUpType::Snapshot => seed_startup_ledger_state(root, options, config),
    }
}

fn load_startup_ledger_from_storage(
    root: &ApplicationRoot,
    options: &AppBootstrapOptions,
) -> Result<(), String> {
    let Some(ledger_master_runtime) = root.ledger_master_runtime() else {
        return Err("Load startup requires an attached LedgerMaster runtime".to_owned());
    };
    let loaded = load_complete_ledger_from_storage(
        root,
        options.start_ledger.as_deref(),
        "app-bootstrap-ledger-loader",
    )?
    .ok_or_else(|| "Requested startup ledger was not found in local storage".to_owned())?;

    hydrate_loaded_ledger(
        root,
        Arc::new(loaded),
        ledger_master_runtime.ledger_master(),
    )?;
    Ok(())
}

fn load_complete_ledger_from_storage(
    root: &ApplicationRoot,
    requested: Option<&str>,
    cache_name: &'static str,
) -> Result<Option<Ledger>, String> {
    let Some(relational) = root.relational_database().as_ref().map(Arc::clone) else {
        return Err(
            "Storage ledger load requires an attached relational ledger database".to_owned(),
        );
    };
    let Some(node_store) = root.node_store().clone() else {
        return Err("Storage ledger load requires an attached NodeStore".to_owned());
    };

    let provider = BootstrapLedgerDbProvider::new(relational);
    let family = SHAMapFamily::new(
        Arc::new(TreeNodeCache::new(
            cache_name,
            8,
            time::Duration::seconds(1),
            MonotonicClock::default(),
        )),
        NullFullBelowCache::new(0),
        BootstrapNodeStoreFetcher::new(node_store),
        NullMissingNodeReporter,
    );
    let journal = NullLedgerJournal;
    let config = LedgerConfig::default();

    let mut loaded = load_bootstrap_ledger(requested, &journal, &config, &family, &provider)?;
    let Some(mut loaded) = loaded.take() else {
        return Ok(None);
    };

    if !loaded.walk_ledger_with_family(&journal, false, &family) {
        return Err(format!(
            "Startup ledger {} is incomplete in local NodeStore",
            loaded.header().seq
        ));
    }
    loaded
        .finish_load_by_index_or_hash(&journal)
        .map_err(|error| format!("startup ledger setup failed: {error:?}"))?;
    loaded.assert_sensible();
    Ok(Some(loaded))
}

fn replay_startup_ledger_from_storage(
    root: &ApplicationRoot,
    options: &AppBootstrapOptions,
) -> Result<(), String> {
    let Some(relational) = root.relational_database().as_ref().map(Arc::clone) else {
        return Err("Replay startup requires an attached relational ledger database".to_owned());
    };
    let Some(node_store) = root.node_store().clone() else {
        return Err("Replay startup requires an attached NodeStore".to_owned());
    };
    let Some(ledger_master_runtime) = root.ledger_master_runtime() else {
        return Err("Replay startup requires an attached LedgerMaster runtime".to_owned());
    };

    let provider = BootstrapLedgerDbProvider::new(relational);
    let family = SHAMapFamily::new(
        Arc::new(TreeNodeCache::new(
            "app-bootstrap-ledger-replay-loader",
            8,
            time::Duration::seconds(1),
            MonotonicClock::default(),
        )),
        NullFullBelowCache::new(0),
        BootstrapNodeStoreFetcher::new(node_store),
        NullMissingNodeReporter,
    );
    let journal = NullLedgerJournal;
    let config = LedgerConfig::default();

    let mut replay_ledger = load_bootstrap_ledger(
        options.start_ledger.as_deref(),
        &journal,
        &config,
        &family,
        &provider,
    )?
    .ok_or_else(|| "Requested replay ledger was not found in local storage".to_owned())?;

    if !replay_ledger.walk_ledger_with_family(&journal, false, &family) {
        return Err(format!(
            "Replay ledger {} is incomplete in local NodeStore",
            replay_ledger.header().seq
        ));
    }
    replay_ledger
        .finish_load_by_index_or_hash(&journal)
        .map_err(|error| format!("replay ledger setup failed: {error:?}"))?;
    replay_ledger.assert_sensible();

    let mut parent_ledger = load_by_hash(
        replay_ledger.header().parent_hash,
        false,
        &journal,
        &config,
        &family,
        &provider,
    )
    .map_err(|error| format!("replay parent ledger load failed: {error:?}"))?
    .ok_or_else(|| "Replay parent ledger was not found in local storage".to_owned())?;

    if !parent_ledger.walk_ledger_with_family(&journal, false, &family) {
        return Err(format!(
            "Replay parent ledger {} is incomplete in local NodeStore",
            parent_ledger.header().seq
        ));
    }
    parent_ledger
        .finish_load_by_index_or_hash(&journal)
        .map_err(|error| format!("replay parent setup failed: {error:?}"))?;
    parent_ledger.assert_sensible();

    let parent = Arc::new(parent_ledger);
    hydrate_loaded_ledger(
        root,
        Arc::clone(&parent),
        ledger_master_runtime.ledger_master(),
    )?;
    inject_replay_transactions(
        root,
        parent,
        Arc::new(replay_ledger),
        &family,
        options.trap_tx_hash,
    )?;
    Ok(())
}

fn load_startup_ledger_from_file(
    root: &ApplicationRoot,
    options: &AppBootstrapOptions,
) -> Result<(), String> {
    let Some(path) = options.start_ledger.as_deref() else {
        return Err("Ledger-file startup requires a file path".to_owned());
    };
    let Some(ledger_master_runtime) = root.ledger_master_runtime() else {
        return Err("Ledger-file startup requires an attached LedgerMaster runtime".to_owned());
    };

    let ledger = load_bootstrap_ledger_from_file(path)?;
    hydrate_loaded_ledger(
        root,
        Arc::new(ledger),
        ledger_master_runtime.ledger_master(),
    )?;
    Ok(())
}

fn load_bootstrap_ledger<P, CLOCK, S, FB, F, MR, NS>(
    requested: Option<&str>,
    journal: &NullLedgerJournal,
    config: &LedgerConfig,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    provider: &P,
) -> Result<Option<Ledger>, String>
where
    P: LedgerInfoProvider,
    CLOCK: basics::tagged_cache::CacheClock,
    S: std::hash::BuildHasher + Clone,
    FB: shamap::family::FullBelowCache,
    F: SHAMapNodeFetcher,
    MR: shamap::family::MissingNodeReporter,
{
    let requested = requested.map(str::trim).filter(|value| !value.is_empty());
    if requested.is_none() || requested == Some("latest") {
        return ledger::get_latest_ledger(journal, config, family, provider)
            .map(|(ledger, _, _)| ledger)
            .map_err(|error| format!("latest local ledger load failed: {error:?}"));
    }

    let requested = requested.expect("requested startup ledger should be present");
    if requested.len() == 64 {
        let hash = Uint256::from_hex(requested)
            .map_err(|_| format!("invalid startup ledger hash: {requested}"))?;
        return load_by_hash(
            basics::sha_map_hash::SHAMapHash::new(hash),
            false,
            journal,
            config,
            family,
            provider,
        )
        .map_err(|error| format!("hash ledger load failed: {error:?}"));
    }

    let ledger_index = requested
        .parse::<u32>()
        .map_err(|_| format!("invalid startup ledger selector: {requested}"))?;
    load_by_index(ledger_index, false, journal, config, family, provider)
        .map_err(|error| format!("indexed ledger load failed: {error:?}"))
}

fn hydrate_loaded_ledger(
    root: &ApplicationRoot,
    ledger: Arc<Ledger>,
    ledger_master: Arc<crate::AppLedgerMaster>,
) -> Result<(), String> {
    let persistence =
        ledger::LedgerPersistence::new(Arc::new(root.build_ledger_persistence_runtime()));
    let ledger = root.ledger_with_node_fetcher(ledger);
    ledger_master.set_closed_ledger(Arc::clone(&ledger));
    ledger_master
        .set_full_ledger(&persistence, Arc::clone(&ledger), true, true, None, None)
        .map_err(|error| format!("ledger master bootstrap failed: {error:?}"))?;
    ledger_master.set_pub_ledger(Arc::clone(&ledger));
    let _ = ledger_master.set_valid_ledger(Arc::clone(&ledger), None, None);

    root.on_closed_ledger(Arc::clone(&ledger));
    root.on_published_ledger(Arc::clone(&ledger));
    let _ = root.on_validated_ledger(Arc::clone(&ledger));

    let next_index = ledger.header().seq.saturating_add(1);
    let base_fee = ledger.fees().base.max(10);
    let _ = root.open_ledger().modify(|view| {
        view.ledger_current_index = next_index;
        view.base_fee_drops = base_fee;
        true
    });

    let _ = root.order_book_db().setup(
        Arc::clone(&ledger),
        Arc::new(NullOrderBookDBRuntime),
        Arc::new(NullOrderBookDBJournal),
    );

    Ok(())
}

fn inject_replay_transactions<CLOCK, S, FB, F, MR, NS>(
    root: &ApplicationRoot,
    parent: Arc<Ledger>,
    replay: Arc<Ledger>,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    trap_tx_hash: Option<Uint256>,
) -> Result<(), String>
where
    CLOCK: basics::tagged_cache::CacheClock,
    S: std::hash::BuildHasher + Clone,
    FB: shamap::family::FullBelowCache,
    F: SHAMapNodeFetcher,
    MR: shamap::family::MissingNodeReporter,
{
    let replay_data = build_replay_data_with_family(parent, replay, family)?;
    let mut found_trap = trap_tx_hash.is_none();

    let _ = root.open_ledger().modify(|view| {
        for tx in replay_data.ordered_txs().values() {
            let tx_id = tx.get_transaction_id();
            if trap_tx_hash.is_some_and(|trap| trap == tx_id) {
                found_trap = true;
            }
            view.push_transaction(tx.clone());
        }
        true
    });

    if !found_trap {
        return Err("Replay ledger does not contain the requested trap transaction".to_owned());
    }

    Ok(())
}

fn build_replay_data_with_family<CLOCK, S, FB, F, MR, NS>(
    parent: Arc<Ledger>,
    replay: Arc<Ledger>,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
) -> Result<LedgerReplay, String>
where
    CLOCK: basics::tagged_cache::CacheClock,
    S: std::hash::BuildHasher + Clone,
    FB: shamap::family::FullBelowCache,
    F: SHAMapNodeFetcher,
    MR: shamap::family::MissingNodeReporter,
{
    let mut ordered_txs = std::collections::BTreeMap::new();
    let mut stack: Vec<NodePathEntry> = Vec::new();
    let mut current = replay
        .tx_map()
        .peek_first_item_with_family(&mut stack, family)
        .map_err(|error| format!("replay tx traversal failed: {error:?}"))?;

    while let Some(node) = current {
        if !node.is_leaf() {
            break;
        }
        let item = node
            .peek_item()
            .ok_or_else(|| "replay tx leaf did not contain an item".to_owned())?;
        let (tx, meta_index) = decode_replay_tx_item(replay.header().seq, &item)?;
        ordered_txs.entry(meta_index).or_insert(tx);
        current = replay
            .tx_map()
            .peek_next_item_with_family(item.key(), &mut stack, family)
            .map_err(|error| format!("replay tx traversal failed: {error:?}"))?;
    }

    Ok(LedgerReplay::new(parent, replay, ordered_txs))
}

fn decode_replay_tx_item(ledger_seq: u32, item: &SHAMapItem) -> Result<(Arc<STTx>, u32), String> {
    let (tx_bytes, meta_bytes) = catch_unwind(AssertUnwindSafe(|| {
        let mut serial = SerialIter::new(item.data());
        (serial.get_vl(), serial.get_vl())
    }))
    .map_err(|_| "failed to split replay transaction-with-meta payload".to_owned())?;

    let tx = catch_unwind(AssertUnwindSafe(|| {
        let mut serial = SerialIter::new(&tx_bytes);
        Arc::new(STTx::from_serial_iter(&mut serial))
    }))
    .map_err(|_| "failed to parse replay STTx".to_owned())?;

    let meta = catch_unwind(AssertUnwindSafe(|| {
        TxMeta::from_raw(item.key(), ledger_seq, &meta_bytes)
    }))
    .map_err(|_| "failed to parse replay TxMeta".to_owned())?;

    Ok((tx, meta.get_index()))
}

fn load_bootstrap_ledger_from_file(path: &str) -> Result<Ledger, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read ledger file {path}: {error}"))?;
    let parsed: serde_json::Value = serde_json::from_str(&contents)
        .map_err(|error| format!("failed to parse ledger JSON {path}: {error}"))?;
    let mut ledger = JsonValue::from(parsed);

    if let Some(result) = ledger.get("result").cloned() {
        ledger = result;
    }
    if let Some(inner) = ledger.get("ledger").cloned() {
        ledger = inner;
    }

    let mut seq = 1u32;
    let mut close_time = 0u32;
    let mut close_time_resolution = 30u8;
    let mut close_time_estimated = false;
    let mut total_drops = 0u64;
    let state_entries = if let Some(account_state) = ledger.get("accountState").cloned() {
        if let Some(index) = ledger.get("ledger_index").and_then(JsonValue::as_u64) {
            seq = index as u32;
        }
        if let Some(file_close_time) = ledger.get("close_time").and_then(JsonValue::as_u64) {
            close_time = file_close_time as u32;
        }
        if let Some(resolution) = ledger
            .get("close_time_resolution")
            .and_then(JsonValue::as_u64)
        {
            close_time_resolution = resolution as u8;
        }
        if let Some(estimated) = ledger.get("close_time_estimated") {
            close_time_estimated = matches!(estimated, JsonValue::Bool(true));
        }
        if let Some(total_coins) = ledger.get("total_coins") {
            total_drops = match total_coins {
                JsonValue::String(value) => value
                    .parse::<u64>()
                    .map_err(|_| "invalid total_coins in ledger file".to_owned())?,
                JsonValue::Unsigned(value) => *value,
                JsonValue::Signed(value) if *value >= 0 => *value as u64,
                _ => return Err("invalid total_coins in ledger file".to_owned()),
            };
        }
        account_state
    } else {
        ledger
    };

    let JsonValue::Array(entries) = state_entries else {
        return Err("ledger file accountState must be an array".to_owned());
    };

    let mut state_tree = MutableTree::new(seq.max(1));
    for entry in entries {
        let JsonValue::Object(mut object) = entry else {
            return Err("invalid entry in ledger file".to_owned());
        };
        let Some(index_text) = object
            .remove("index")
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
        else {
            return Err("ledger file entry missing index".to_owned());
        };
        let index = Uint256::from_hex(&index_text)
            .map_err(|_| format!("invalid ledger entry index in {path}"))?;
        let sle = if let Some(blob_text) = object
            .remove("blob")
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
        {
            let bytes = str_unhex(&blob_text)
                .ok_or_else(|| format!("invalid ledger entry blob in {path}"))?;
            let mut iter = SerialIter::new(&bytes);
            let entry = STLedgerEntry::from_serial_iter(&mut iter, index);
            if !iter.empty() {
                return Err(format!(
                    "invalid trailing bytes in ledger entry blob {path}"
                ));
            }
            entry
        } else {
            let parsed = STParsedJSONObject::new("sle", &JsonValue::Object(object));
            let st_object = parsed
                .object
                .ok_or_else(|| format!("invalid ledger file entry in {path}"))?;
            STLedgerEntry::from_stobject(st_object, index)
        };
        state_tree
            .add_item(
                SHAMapNodeType::AccountState,
                SHAMapItem::new(index, sle.get_serializer().data().to_vec()),
            )
            .map_err(|error| format!("failed to add ledger file entry: {error:?}"))?;
    }

    let mut ledger = Ledger::from_maps(
        LedgerHeader {
            seq,
            close_time,
            close_time_resolution,
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            state_tree.root(),
            SHAMapType::State,
            false,
            seq,
            SyncState::Modifying,
        ),
        SyncTree::new_with_type(SHAMapType::Transaction, false, seq),
    );
    ledger.set_total_drops(total_drops);
    let _ = ledger
        .set_accepted_and_setup_from_config(
            close_time,
            close_time_resolution,
            !close_time_estimated,
            &LedgerConfig::default(),
        )
        .map_err(|error| format!("failed to finalize ledger file state: {error:?}"))?;
    Ok(ledger)
}

fn parse_sql_hash(value: String) -> rusqlite::Result<basics::sha_map_hash::SHAMapHash> {
    Uint256::from_hex(&value)
        .map(basics::sha_map_hash::SHAMapHash::new)
        .map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                value.len(),
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::other("invalid ledger hash")),
            )
        })
}

fn to_xrpld_startup_type(start_type: StartUpType) -> xrpld_core::StartUpType {
    match start_type {
        StartUpType::Fresh => xrpld_core::StartUpType::Fresh,
        StartUpType::Normal => xrpld_core::StartUpType::Normal,
        StartUpType::Load => xrpld_core::StartUpType::Load,
        StartUpType::LoadFile => xrpld_core::StartUpType::LoadFile,
        StartUpType::Replay => xrpld_core::StartUpType::Replay,
        StartUpType::Network => xrpld_core::StartUpType::Network,
        StartUpType::Snapshot => xrpld_core::StartUpType::Snapshot,
    }
}

fn seed_startup_ledger_state(
    root: &ApplicationRoot,
    options: &AppBootstrapOptions,
    config: &BasicConfig,
) -> Result<(), String> {
    let seed_seq = options
        .start_ledger
        .as_deref()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|seq| *seq > 0)
        .unwrap_or(1);
    let backed = root.node_store().is_some();

    let closed = match options.start_type {
        StartUpType::Fresh | StartUpType::Network | StartUpType::Snapshot => {
            // Enable amendments at genesis matching reference rippled --start.
            // If [amendments] is configured, use those IDs (matching rippled
            // which reads its [amendments] section to determine getDesired()).
            // Otherwise fall back to all supported + DefaultYes features.
            let genesis_amendments = amendments_from_config(config);
            Ledger::create_genesis(backed, &LedgerConfig::default(), genesis_amendments)
                .unwrap_or_else(|_| Ledger::from_ledger_seq_and_close_time(1, 0, backed))
        }
        StartUpType::Replay => {
            Ledger::from_ledger_seq_and_close_time(seed_seq.max(2) - 1, 0, backed)
        }
        StartUpType::Load | StartUpType::LoadFile => {
            Ledger::from_ledger_seq_and_close_time(seed_seq, 0, backed)
        }
        StartUpType::Normal => Ledger::from_ledger_seq_and_close_time(seed_seq.max(1), 0, backed),
    };
    let closed = Arc::new(closed);
    tracing::info!(target: "bootstrap", ledger_seq = closed.header().seq, "Genesis ledger loaded");
    let hydrate_seed_as_loaded = !matches!(
        options.start_type,
        StartUpType::Fresh | StartUpType::Network
    );
    if hydrate_seed_as_loaded
        && closed.is_immutable()
        && let Some(ledger_master_runtime) = root.ledger_master_runtime()
    {
        hydrate_loaded_ledger(
            root,
            Arc::clone(&closed),
            ledger_master_runtime.ledger_master(),
        )?;
        return Ok(());
    }

    root.on_closed_ledger(Arc::clone(&closed));
    root.on_published_ledger(Arc::clone(&closed));

    // ledger header to SQLite so that subsequent loads can find it.
    if let Some(relational) = root.relational_database() {
        if let Ok(accepted) = ledger::AcceptedLedger::new(Arc::clone(&closed)) {
            let _ = relational.write_accepted_ledger(
                &accepted,
                &root.transaction_master(),
                root.network_id(),
            );
        }
    }

    // Only mark the genesis ledger as validated when explicitly requested
    // (standalone / --valid mode) or when loading a specific ledger from
    // local storage.  For a normal network start (Fresh or Network startup
    // type without --valid), the genesis ledger must NOT be pre-validated:
    // the node must wait for real network validations before promoting its
    // validated-ledger pointer.  Marking genesis validated here is what
    // caused the premature `validated_ledger.seq=1` and the early
    // `tracking` state promotion that blocked ledger resolution.
    //
    // `switchLCL()` — it never calls `setValidLedger()` for network nodes.
    if options.start_valid
        || matches!(
            options.start_type,
            StartUpType::Load | StartUpType::LoadFile | StartUpType::Replay
        )
    {
        let _ = root.on_validated_ledger(Arc::clone(&closed));
    }

    let next_index = closed.header().seq.saturating_add(1);
    let _ = root.open_ledger().modify(|view| {
        view.ledger_current_index = next_index;
        if view.base_fee_drops == 0 {
            view.base_fee_drops = 10;
        }
        true
    });

    Ok(())
}

fn amendments_from_config(config: &BasicConfig) -> Vec<Uint256> {
    let section = config.section("amendments");
    let values = section.values();
    if !values.is_empty() {
        return values
            .iter()
            .filter_map(|line| {
                let hex = line.split_whitespace().next()?;
                if hex.len() != 64 {
                    return None;
                }
                let bytes = str_unhex(hex)?;
                Uint256::from_slice(&bytes)
            })
            .collect();
    }
    // Fallback: all supported amendments voted DefaultYes.
    REGISTERED_FEATURES
        .iter()
        .filter(|f| f.supported && f.vote == RegisteredFeatureVote::DefaultYes)
        .map(|f| feature_id(f.name))
        .collect()
}

fn config_legacy_u32(config: &BasicConfig, section: &str) -> Option<u32> {
    let value = config.legacy(section).ok()?;
    let trimmed = value.trim();
    match trimmed.to_ascii_lowercase().as_str() {
        "full" => Some(u32::MAX),
        "none" => Some(0),
        _ => trimmed.parse::<u32>().ok(),
    }
}

fn config_legacy_usize(config: &BasicConfig, section: &str) -> Option<usize> {
    config.legacy(section).ok()?.trim().parse::<usize>().ok()
}

fn config_path_search_max(config: &BasicConfig) -> u32 {
    if let Some(explicit) = config_legacy_u32(config, "path_search_max") {
        return explicit;
    }

    if config.exists("validation_seed") || config.exists("validator_token") {
        0
    } else {
        3
    }
}

fn parse_basic_config_text(text: &str) -> Result<BasicConfig, String> {
    let mut sections = IniFileSections::new();
    let mut current_section = String::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].trim().to_owned();
            let _ = sections.entry(current_section.clone()).or_default();
            continue;
        }

        sections
            .entry(current_section.clone())
            .or_default()
            .push(raw_line.to_owned());
    }

    let mut config = BasicConfig::new();
    config.build(&sections);
    Ok(config)
}

fn usage() -> String {
    [
        "usage: xrpld [options] <command> <params>",
        "General Options:",
        "  --conf PATH         Specify the configuration file.",
        "  --debug             Enable normally suppressed debug logging",
        "  --definitions       Output server definitions as JSON and exit.",
        "  --help, -h          Display this message.",
        "  --newnodeid         Generate a new node identity for this server.",
        "  --nodeid ID         Specify the node identity for this server.",
        "  --quorum N          Override the minimum validation quorum.",
        "  --silent            No output to the console after startup.",
        "  --standalone, -a    Run with no peers.",
        "  --verbose, -v       Verbose logging.",
        "  --version           Display the build version.",
        "",
        "Ledger/Data Options:",
        "  --force_ledger_present_range MIN,MAX",
        "                      Specify the range of present ledgers for testing.",
        "  --import            Import an existing node database.",
        "  --ledger ID         Load the specified ledger and start from the value given.",
        "  --ledgerfile PATH   Load the specified ledger file.",
        "  --load              Load the current ledger from the local DB.",
        "  --net               Get the initial ledger from the network.",
        "  --replay            Replay a ledger close.",
        "  --trap_tx_hash HASH Trap a specific transaction during replay.",
        "  --start             Start from a fresh Ledger.",
        "  --vacuum            VACUUM the transaction db.",
        "  --valid             Consider the initial ledger a valid network ledger.",
        "",
        "RPC Client Options:",
        "  --rpc               Perform rpc command. Assumed if any positional parameters provided.",
        "  --rpc_ip IP[:PORT]  Specify the IP address for RPC command.",
        "  --rpc_port PORT     Specify the port number for RPC command.",
        "",
        "Unit Test Options:",
        "  --quiet, -q         Suppress test suite messages.",
        "  --unittest [SEL]    Perform unit tests.",
        "  --unittest-arg ARG  Supplies an argument string to unit tests.",
        "  --unittest-ipv6     Use IPv6 localhost when running unittests.",
        "  --unittest-log      Force unit test log message output.",
        "  --unittest-jobs N   Number of unittest jobs to run in parallel.",
    ]
    .join("\n")
}

struct SystemDescriptorLimitProvider;

impl DescriptorLimitProvider for SystemDescriptorLimitProvider {
    fn current_descriptor_limit(&self) -> Option<u64> {
        #[cfg(unix)]
        {
            use libc::{RLIM_INFINITY, RLIMIT_NOFILE, getrlimit, rlimit};
            let mut limits = rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            let status = unsafe { getrlimit(RLIMIT_NOFILE, &mut limits) };
            if status != 0 || limits.rlim_cur == RLIM_INFINITY {
                return None;
            }
            Some(limits.rlim_cur)
        }

        #[cfg(not(unix))]
        {
            None
        }
    }

    fn set_descriptor_limit(&self, requested: u64) -> Option<u64> {
        #[cfg(unix)]
        {
            use libc::{RLIMIT_NOFILE, getrlimit, rlimit, setrlimit};
            let mut limits = rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if unsafe { getrlimit(RLIMIT_NOFILE, &mut limits) } != 0 {
                return None;
            }
            limits.rlim_cur = requested;
            if unsafe { setrlimit(RLIMIT_NOFILE, &limits) } != 0 {
                return None;
            }
            Some(requested)
        }

        #[cfg(not(unix))]
        {
            let _ = requested;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MainRuntime, spawn_shutdown_watcher};
    use crate::ApplicationRoot;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn shutdown_watcher_exits_when_stop_is_already_requested() {
        let runtime = Arc::new(MainRuntime::new(
            ApplicationRoot::new(0).expect("root should build"),
        ));
        let stop_requested = Arc::new(AtomicBool::new(true));

        let handle = spawn_shutdown_watcher(runtime, stop_requested);
        handle.join().expect("watcher should exit cleanly");
    }
}
