//! App-owned bootstrap assembly for the migrated runtime shell.
//!
//! This stays inside the app crate and only assembles the pieces that the app
//! crate can truthfully own today: config loading, `ApplicationRoot` setup,
//! default node-family ownership, optional SHAMap store ownership, and the
//! `MainRuntime` shell.

use crate::{
    ApplicationRoot, ApplicationRootOptions, BootstrapOverlayHandoff, DescriptorLimitProvider,
    LedgerReplay, MainRuntime, SHAMapStoreComponent,
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
    JsonValue, REGISTERED_FEATURES, RegisteredFeatureVote, STLedgerEntry, STParsedJSONObject, STTx,
    SerialIter, TxMeta, feature_id,
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
use std::sync::Mutex;
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
            io_threads: 6,
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
        &self,
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
    // - standalone → Full (node operates without network)
    // - start_valid → Full (node starts fully synced)
    // - non-standalone → Connected (node starts connected to network)
    {
        use crate::network::network_ops::NetworkOpsOperatingMode;
        let mode = if options.standalone || options.start_valid {
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
    // Configure TxQ for standalone mode (higher min_txn prevents fee escalation).
    if options.standalone {
        root.tx_q().set_standalone(true);
    }
    // Apply [transaction_queue] config overrides.
    let txq_setup = parse_txq_setup(config);
    if config.exists("transaction_queue") {
        root.tx_q().reconfigure_setup(txq_setup);
    }
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

    // Standalone mode operates without network peers — skip overlay entirely.
    if !options.standalone {
        let _ =
            root.attach_configured_overlay_runtime(config, Arc::new(BootstrapOverlayHandoff))?;
    }

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
            root.validators()
                .update_trusted(&std::collections::HashSet::new(), 0);
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
    let standalone = runtime.root().standalone();
    ensure_descriptor_budget(bootstrap.report.fd_required)?;
    runtime.start()?;
    tracing::info!(target: "app", "Node startup complete");

    // For --start mode: `root.on_closed_ledger` (called during genesis
    // ledger load, see `build_bootstrap_root`) already seeded
    // `ApplicationRoot`'s single closed-ledger tracker with the genesis
    // ledger, so consensus can find it as a parent. The first round is
    // started in the event loop once peers are connected (so proposals
    // arrive before the idle timeout closes it).

    // Standalone mode: no overlay, no consensus thread. The node operates in
    // Full mode with the genesis ledger as validated. Ledger advancement is
    // driven exclusively by `ledger_accept` RPC calls.
    if standalone {
        tracing::info!(
            target: "app",
            validated_seq = runtime.root().validated_ledger_seq(),
            "Standalone mode active — no peers, no consensus. Use ledger_accept to advance."
        );

        let stop_requested = Arc::new(AtomicBool::new(false));
        let stop_thread =
            spawn_shutdown_watcher(Arc::clone(&runtime), Arc::clone(&stop_requested));

        runtime.run();

        stop_requested.store(true, Ordering::Release);
        let _ = stop_thread.join();
        return Ok(());
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

    // Spawn JobQueue worker threads (matches rippled's JobQueue thread pool).
    // Without these, jobs added via add_job() (e.g. RPC-submitted transactions
    // routed through submit_transaction_to_network_ops) sit in the queue
    // forever and never reach process_transaction, so they never enter the
    // open ledger's transaction set or get included in consensus.
    {
        let jq_template = runtime.root().job_queue();
        let worker_count = jq_template.worker_thread_count().max(1);
        for i in 0..worker_count {
            let jq = jq_template.clone();
            std::thread::Builder::new()
                .name(format!("jobqueue-worker-{i}"))
                .spawn(move || {
                    jq.run_worker_loop();
                })
                .expect("failed to spawn jobqueue worker thread");
        }
    }
    let consensus_thread = if matches!(
        bootstrap.report.startup_ledger_mode,
        StartUpType::Fresh | StartUpType::Network
    ) && bootstrap.report.has_overlay_runtime
    {
        // This thread exclusively drives consensus in --start mode.
        // For genesis (seq=1), start immediately like rippled (no waiting).
        // For joining an existing network (seq>1), wait for peer confirmation.
        let is_genesis = runtime
            .root()
            .closed_ledger()
            .map(|l| l.header().seq <= 1)
            .unwrap_or(false);
        if !is_genesis {
            runtime.root().set_need_network_ledger(true);
        }
        let stop_flag = Arc::clone(&consensus_stop);
        let rt = Arc::clone(&runtime);
        Some(
            std::thread::Builder::new()
                .name("start-mode-consensus".into())
                .spawn(move || {
                    run_start_mode_consensus_loop(rt.clone(), stop_flag.clone());
                })
                .expect("failed to spawn start-mode-consensus thread"),
        )
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
fn run_start_mode_consensus_loop(runtime: Arc<MainRuntime>, stop: Arc<AtomicBool>) {
    use consensus;

    // Elevate thread priority — consensus must never be starved by RPC load.
    #[cfg(unix)]
    unsafe {
        libc::setpriority(0, 0, -15);
        #[cfg(target_os = "linux")]
        {
            let param = libc::sched_param { sched_priority: 10 };
            libc::pthread_setschedparam(libc::pthread_self(), libc::SCHED_RR, &param);
        }
    }

    tracing::info!(target: "consensus", "Start-mode consensus event loop running (elevated priority)");

    // Take the map-complete receiver once (it's a take-once resource).
    let map_complete_rx = runtime
        .root()
        .consensus_runtime()
        .and_then(|cr| cr.take_map_complete_receiver());

    let is_genesis = !runtime.root().need_network_ledger();
    let mut consensus_started = false;
    let mut last_timer_tick = std::time::Instant::now();
    let mut last_round_ledger_id: Option<Uint256> = None;
    let mut last_acquire_tick = std::time::Instant::now();
    let mut last_history_tick = std::time::Instant::now();
    if is_genesis {
        tracing::info!(target: "consensus", "Genesis mode: starting consensus immediately on the genesis ledger");
    }

    // Consensus event channel: validations and completed ledgers feed into the
    // driver event loop which handles checkAccept + ledger promotion.
    let (event_tx, event_rx) =
        crate::consensus::driver::consensus_event_channel();

    let (shared_completed_tx, shared_completed_rx) = std::sync::mpsc::channel::<Arc<ledger::Ledger>>();
    let lm_rt_for_shared_inbound = runtime.root().ledger_master_runtime();
    let shared_inbound = lm_rt_for_shared_inbound
        .as_ref()
        .and_then(|lm_rt| lm_rt.shared_inbound_ledgers.lock().ok()?.clone())
        .unwrap_or_else(|| {
            Arc::new(crate::ledger::shared_inbound_ledgers::SharedInboundLedgers::new(
                Arc::new(Mutex::new(std::collections::HashMap::new())),
                Arc::new(shamap::tree_node_cache::TreeNodeCache::new(
                    "driver-tc",
                    1024,
                    time::Duration::seconds(60),
                    basics::tagged_cache::MonotonicClock::default(),
                )),
                Arc::new(shamap::family::FullBelowCacheImpl::new(
                    0,
                    basics::tagged_cache::MonotonicClock::default(),
                    basics::hardened_hash::HardenedHashBuilder::default(),
                    1024,
                )),
                Arc::new(ledger::FetchPackCache::new(
                    256,
                    time::Duration::seconds(120),
                    basics::tagged_cache::MonotonicClock::default(),
                )),
                Arc::new(crate::ledger::shared_inbound_ledgers::RunDataLimiter::new(4)),
                Arc::new(basics::tagged_cache::KeyCache::new(
                    "driver-dedup",
                    1024,
                    time::Duration::seconds(30),
                    basics::tagged_cache::MonotonicClock::default(),
                )),
                shared_completed_tx.clone(),
            ))
        });
    // Store the (possibly freshly constructed) instance back into
    // `AppLedgerMasterRuntime` so every OTHER consumer -- most importantly
    // `AppRclConsensusAdaptor::acquire_ledger`, which needs to actively
    // dispatch a fetch for a ledger consensus doesn't have yet (matching
    // the reference's `RCLConsensus::Adaptor::acquireLedger` calling
    // `app.getInboundLedgers().acquireAsync(id, 0, Reason::CONSENSUS)`
    // when the cache lookup misses) -- sees and uses the SAME instance
    // this loop's `spawn_event_loop`/`shared_completed_rx` are wired to.
    // Without this, a second, throwaway `SharedInboundLedgers` would be
    // constructed by whoever calls `acquire_ledger` next, whose
    // completions would never reach this loop's `ledger_history()` insert
    // (they'd arrive on a *different*, disconnected `shared_completed_tx`).
    if let Some(lm_rt) = lm_rt_for_shared_inbound.as_ref()
        && let Ok(mut guard) = lm_rt.shared_inbound_ledgers.lock()
        && guard.is_none()
    {
        *guard = Some(Arc::clone(&shared_inbound));
    }

    // Wire the shared acquisition instance to a real node-store write
    // pipeline and the overlay, matching `xrpld/main`'s own standalone
    // catchup wiring (`inbound_ledgers.set_node_store`/`set_write_tx`/
    // `set_pending_writes`/`set_overlay_rt`). Without this,
    // `SharedInboundLedgers::acquire` early-returns unconditionally on its
    // internal `node_store`/`write_tx`/`pending_writes` guards, silently
    // no-opping every active-acquisition request -- this was a genuine,
    // previously-undiscovered gap: `--start` mode's `AppRclConsensusAdaptor
    // ::acquire_ledger` only ever did a passive cache lookup and never had
    // a way to actively catch a lagging node up to a ledger it doesn't
    // have, because this whole pipeline was never connected.
    if let Some(ns) = runtime.root().node_store().as_ref() {
        shared_inbound.set_node_store(ns.clone());
        let pending_writes = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let (write_tx, _write_handle) = crate::ledger::shared_inbound_ledgers::spawn_nodestore_writer(ns.clone(), Arc::clone(&pending_writes));
        shared_inbound.set_write_tx(write_tx);
        shared_inbound.set_pending_writes(pending_writes);
    }
    if let Some(overlay_rt) = runtime.root().overlay_runtime() {
        // Wire InboundTransactions with a live-peer PeerSetBuilder so
        // TransactionAcquire can actually send requests to peers.
        // At registry construction time the overlay doesn't exist yet
        // (SimplePeerSetBuilder::new(Vec::new())), so this must be done
        // once the overlay is confirmed ready.
        {
            let mut guard = runtime
                .root()
                .inbound_transactions()
                .lock()
                .expect("inbound_transactions mutex");
            guard.set_peer_set_builder(Arc::new(
                overlay::OverlayPeerSetBuilder::new(overlay_rt.overlay()),
            ));
        }
        shared_inbound.set_overlay_rt(overlay_rt);
    }

    let event_loop_app = runtime.root().clone();
    let event_loop_stop = Arc::clone(&stop);
    crate::consensus::driver::spawn_event_loop(
        event_loop_app,
        Arc::clone(&shared_inbound),
        event_rx,
        event_loop_stop,
    );

    // Validation forwarding thread: receives overlay notify signal, takes
    // validations, and forwards them as ConsensusEvent::Validation to the
    // event loop. This bridges the overlay's SyncSender<()> notify pattern
    // to the unified event channel.
    {
        let (val_notify_tx, val_notify_rx) = std::sync::mpsc::sync_channel::<()>(1);
        if let Some(overlay_rt) = runtime.root().overlay_runtime() {
            overlay_rt
                .overlay()
                .queued_inbound()
                .set_validation_notify(val_notify_tx);
        }
        let fwd_stop = Arc::clone(&stop);
        let fwd_runtime = Arc::clone(&runtime);
        let fwd_event_tx = event_tx.clone();
        std::thread::Builder::new()
            .name("validation-forwarder".into())
            .spawn(move || {
                loop {
                    let _ = val_notify_rx.recv();
                    if fwd_stop.load(Ordering::Acquire) {
                        break;
                    }
                    let root = fwd_runtime.root();
                    let Some(overlay_rt) = root.overlay_runtime() else { continue; };
                    let validations = overlay_rt.overlay().take_validations();
                    for queued in validations {
                        if fwd_event_tx
                            .send(crate::consensus::driver::ConsensusEvent::Validation(queued))
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            })
            .expect("spawn validation-forwarder thread");
    }

    // Immediate transaction dispatch — matches reference
    // `PeerImp::handleTransaction` calling
    // `JobQueue::addJob(JtTransaction, "RcvCheckTx", ...)` synchronously on
    // receipt from the network thread. Without this, `--start` mode never
    // registers a transaction router on `QueuedOverlayInboundHandler`, so
    // every inbound `TMTransaction` from a peer gets queued into
    // `OverlayInboundSnapshot::transactions` and is NEVER consumed or
    // applied -- confirmed via a live cluster test where a burst of
    // transactions submitted to one node's RPC never appeared on ANY other
    // node's ledger, even though the peer network delivered the raw
    // message (peers recognized the transaction's hash via `tx` lookups,
    // just permanently unvalidated). This meant nodes could only ever
    // include transactions submitted directly to their own RPC endpoint,
    // making network-wide agreement on a shared transaction set impossible
    // except by coincidence. Mirrors `main.rs`'s standalone catchup wiring
    // (`process_inbound_transaction`) exactly: parse the raw bytes back
    // into an `STTx`, wrap it, and run it through the SAME
    // Transaction relay: process_transaction on network thread immediately.
    // This adds each relayed tx to the NetworkOps pending queue as fast as
    // possible (sub-ms). The open-ledger modify happens in ONE batch in the
    // drain block (50ms) and in on_close (before capture). This matches
    // rippled's doTransactionAsync: adds to transactions_ queue on network
    // thread, then transactionBatch/apply() does one openLedger.modify().
    if let Some(overlay_rt) = runtime.root().overlay_runtime() {
        let router_root = runtime.root().clone();
        overlay_rt.overlay().queued_inbound().set_transaction_router(Box::new(move |_peer_id, message| {
            let mut serial = protocol::SerialIter::new(&message.message.raw_transaction);
            let st_tx = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                protocol::STTx::from_serial_iter(&mut serial)
            })) {
                Ok(tx) => tx,
                Err(_) => return,
            };
            let st_tx = Arc::new(st_tx);
            let mut transaction: crate::SharedTransaction = Arc::new(std::sync::Mutex::new(
                crate::tx_queue::transaction::Transaction::new(Arc::clone(&st_tx)),
            ));
            if let Some(network_ops_runtime) = router_root.network_ops_runtime() {
                let _ = network_ops_runtime.process_transaction(
                    &mut transaction,
                    false,
                    false,
                    false,
                    || false,
                    || {},
                );
                // Do NOT call apply_pending here. Relay transactions are
                // staged in the shared pending queue and applied by the SAME
                // apply_pending call that processes local (RPC) transactions.
                // This ensures ONE openLedger.modify() processes all pending
                // (local + relay) together — same TxQ fee escalation decisions
                // on all nodes, matching rippled's transactionBatch/apply().
                tracing::info!(target: "app", "DIAG router: relay tx process_transaction done, notifying batch");
                router_root.notify_tx_pending();
            }
        }));
    }

    // Route ALL incoming TmLedgerData responses from the network thread,
    // matching rippled's PeerImp::onMessage(TMLedgerData) which dispatches
    // to both InboundLedgers::gotLedgerData (types 0/1/2) and
    // InboundTransactions::gotData (type 3) synchronously.
    if let Some(overlay_rt) = runtime.root().overlay_runtime() {
        let router_root = runtime.root().clone();
        let router_overlay = overlay_rt.overlay();
        let router_shared_inbound = Arc::clone(&shared_inbound);
        overlay_rt.overlay().queued_inbound().set_ledger_data_router(Box::new(move |peer_id, message| {
            use overlay::Overlay;
            tracing::trace!(target: "consensus", r#type = message.r#type, hash_len = message.ledger_hash.len(), "router_callback ENTRY");
            let Some(hash) = Uint256::from_slice(&message.ledger_hash) else { return; };

            match message.r#type {
                3 => {
                    // liTS_CANDIDATE: route to InboundTransactions for
                    // tx-set dispute resolution during consensus.
                    let peer = router_overlay.find_peer_by_short_id(peer_id);
                    let mut guard = router_root
                        .inbound_transactions()
                        .lock()
                        .expect("inbound_transactions mutex");
                    let status = guard.got_data(hash, peer, &message);
                    tracing::info!(target: "consensus",
                        %hash,
                        nodes_count = message.nodes.len(),
                        status = ?status,
                        "ledger_data_router: type-3 response received"
                    );
                    if let Some(acquire) = guard.acquire(hash) {
                        let complete = acquire.is_complete();
                        let failed = acquire.is_failed();
                        tracing::info!(target: "consensus", %hash, complete, failed, "ledger_data_router: acquire state after got_data");
                        if complete {
                            let set = Arc::new(acquire.map().clone());
                            guard.give_set(hash, set, true);
                        }
                    } else {
                        tracing::info!(target: "consensus", %hash, "ledger_data_router: no active acquire for this hash");
                    }
                }
                0 | 1 | 2 => {
                    // li_BASE / liTX_NODE / liAS_NODE: route to
                    // SharedInboundLedgers for ledger catchup/acquisition.
                    let packet_type = match message.r#type {
                        0 => ledger::InboundLedgerDataType::Base,
                        1 => ledger::InboundLedgerDataType::TransactionNode,
                        _ => ledger::InboundLedgerDataType::StateNode,
                    };
                    let nodes: Vec<ledger::InboundLedgerNodeData> = message
                        .nodes
                        .iter()
                        .map(|n| ledger::InboundLedgerNodeData::new(
                            n.nodeid.clone(),
                            n.nodedata.clone(),
                        ))
                        .collect();
                    let packet = ledger::InboundLedgerPacket::new(packet_type, nodes);
                    router_shared_inbound.route_response(&hash, peer_id as u64, packet);
                }
                _ => {}
            }
        }));
    }

    // === BATCH APPLY THREAD (matches rippled's JtBatch worker) ===
    // Dedicated thread whose ONLY job is: wake on notify_tx_pending →
    // drain overlay queue → process → apply_network_ops_pending → sleep.
    // Runs independently from the consensus timer (1s heartbeat below).
    // This gives ~995ms of headroom: by the time the timer fires,
    // this thread has already applied ALL relay from the past second.
    let batch_root = runtime.root().clone();
    let batch_overlay = runtime.root().overlay_runtime().map(|rt| rt.overlay().clone());
    let batch_network_ops: Option<Arc<crate::network::network_ops_runtime::AppNetworkOpsRuntime>> = runtime.root().network_ops_runtime();
    let batch_stop = Arc::clone(&stop);
    let _batch_thread = std::thread::Builder::new()
        .name("tx-batch-apply".to_string())
        .spawn(move || {
            while !batch_stop.load(Ordering::Acquire) {
                // Block until relay arrives (or 50ms timeout as fallback)
                batch_root.wait_tx_or_timeout(Duration::from_millis(50));

                if batch_stop.load(Ordering::Acquire) {
                    break;
                }

                // Loop until pending is truly empty — matches rippled's
                // transactionBatch: while(!transactions_.empty()) { apply(); }
                // This ensures we never fall behind the arrival rate.
                loop {
                    // Drain overlay queue → process each
                    if let Some(ref overlay) = batch_overlay {
                        let relayed = overlay.take_transactions();
                        if let Some(ref network_ops_rt) = batch_network_ops {
                            for message in relayed {
                                let mut serial = protocol::SerialIter::new(&message.message.raw_transaction);
                                let st_tx = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    protocol::STTx::from_serial_iter(&mut serial)
                                })) {
                                    Ok(tx) => tx,
                                    Err(_) => continue,
                                };
                                let st_tx = Arc::new(st_tx);
                                let mut transaction: crate::SharedTransaction = Arc::new(std::sync::Mutex::new(
                                    crate::tx_queue::transaction::Transaction::new(Arc::clone(&st_tx)),
                                ));
                                let _ = network_ops_rt.process_transaction(
                                    &mut transaction,
                                    false,
                                    false,
                                    false,
                                    || false,
                                    || {},
                                );
                            }
                        }
                    }

                    // Apply batch
                    let report = batch_root.apply_network_ops_pending_to_open_ledger();
                    let applied = report.as_ref().map_or(0, |r| r.entries.len());

                    // If nothing was applied AND overlay queue is empty, we're caught up
                    let overlay_empty = batch_overlay.as_ref()
                        .map_or(true, |o| o.queued_inbound().transaction_count() == 0);
                    if applied == 0 && overlay_empty {
                        break;
                    }
                }
            }
        })
        .expect("failed to spawn tx-batch-apply thread");

    // Wire up instant-wake notification: when overlay queues a relay tx
    // (no router set), wake the batch thread immediately.
    if let Some(overlay_rt) = runtime.root().overlay_runtime() {
        let notify_root = runtime.root().clone();
        overlay_rt.overlay().queued_inbound().set_transaction_notify(Box::new(move || {
            notify_root.notify_tx_pending();
        }));
    }

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

        // Genesis private networks start consensus on the genesis ledger
        // immediately -- there is no network ledger to acquire first, and
        // no peer-confirmation gate to wait on (matching rippled's own
        // standalone/fresh-network behavior of calling `beginConsensus` as
        // soon as the first heartbeat fires). This must run exactly once,
        // before the first `timer_tick`, so `Consensus::startRound`
        // establishes the initial round that `timer_tick` then drives
        // forward. Reads `root.closed_ledger()` -- `ApplicationRoot`'s
        // `SharedLedgerMasterState`, the single source of truth for "the
        // closed ledger" everywhere in this loop (matching the reference's
        // single `LedgerMaster::closedLedger_`; there is no second tracker
        // read from anywhere in this function).
        if is_genesis && !consensus_started {
            if let Some(closed) = root.closed_ledger() {
                if network_ops_rt
                    .maybe_begin_consensus_from_validated(consensus_rt.as_ref(), Arc::clone(&closed))
                {
                    tracing::info!(target: "consensus",
                        seq = closed.header().seq,
                        "Consensus started on genesis ledger"
                    );
                    consensus_started = true;
                    // Clear main.js's transaction router so relayed txns
                    // accumulate in the overlay queue for single-strand processing.
                    overlay_rt.overlay().queued_inbound().clear_transaction_router();
                    last_round_ledger_id = Some(*closed.header().hash.as_uint256());
                    last_timer_tick = std::time::Instant::now();
                }
            }
        }

        // Once a round reaches `Accepted`, `Consensus::timer_entry` becomes
        // a permanent no-op (it early-returns in that phase) until a new
        // round is started. Matches the reference's `NetworkOPsImp::onAccept`
        // job unconditionally calling `endConsensus` -> `beginConsensus` ->
        // `Consensus::startRound` after building the new ledger; without an
        // equivalent re-trigger here, the chain would build exactly one
        // ledger and then stall forever on every subsequent tick. Detected
        // by polling `root.closed_ledger()`'s id: once `on_accept`'s
        // spawned job (`AppRclConsensusAdaptor::on_accept` ->
        // `ApplicationRoot::accept_ledger` -> `on_closed_ledger`) advances
        // it past whatever id consensus most recently started its round
        // with, immediately start the next round on that new ledger.
        if consensus_started && consensus_rt.phase() == consensus::algorithm::ConsensusPhase::Accepted {
            if let Some(closed) = root.closed_ledger() {
                let closed_id = *closed.header().hash.as_uint256();
                if last_round_ledger_id != Some(closed_id) {
                    network_ops_rt.start_next_round(consensus_rt.as_ref(), Arc::clone(&closed));
                    tracing::info!(target: "consensus",
                        seq = closed.header().seq,
                        "Consensus started next round on newly accepted ledger"
                    );
                    last_round_ledger_id = Some(closed_id);
                    last_timer_tick = std::time::Instant::now();
                }
            }
        }

        // Fire consensus tick if >=1s elapsed since last tick.
        // Matches rippled's `JtNetopTimer` heartbeat at ledgerGRANULARITY=1s.
        // This is a SEPARATE cadence from the batch-apply thread — by the time
        // this fires, the batch thread has already applied ALL relay transactions
        // that arrived in the past ~995ms, giving natural headroom for all nodes
        // to converge on the same open ledger state before shouldCloseLedger fires.
        macro_rules! maybe_tick_consensus {
            () => {
                if consensus_started && last_timer_tick.elapsed() >= Duration::from_secs(1) {
                    network_ops_rt.handle_consensus_timer(consensus_rt.as_ref());
                    last_timer_tick = std::time::Instant::now();
                }
            };
        }

        // Serve TmGetLedger requests from peers (PART 1). Matches the
        // reference's `PeerImp::onMessage(TMGetLedger)` dispatching via
        // `app_.getJobQueue().addJob(JtLedgerReq, "RcvGetLedger", ...)`
        // instead of servicing the (potentially disk-touching) SHAMap node
        // walk inline on the network/consensus-timer thread. Previously
        // this ran synchronously here, sharing this loop's thread with
        // Consensus tick first, so on_close/give_set fires before we
        // serve any liTS_CANDIDATE requests (which need the stored set).
        maybe_tick_consensus!();

        // Serve peer ledger/tx-set requests AFTER the consensus tick,
        // ensuring any tx-set stored by on_close during this tick is
        // available for get_set lookups in the itype=3 handler.
        dispatch_get_ledger_requests(root, &overlay_rt);

        // Broadcast our closed ledger to peers via StatusChange so they
        // can detect whether we're at genesis or ahead. This replaces the
        // overlay timer StatusChange that doesn't run in --start mode.
        // Matches the reference's `notify(neACCEPTED_LEDGER, ...)`, called
        // once here for the pre-consensus-start bootstrap window; once
        // consensus is running, `ConsensusLedgerAcceptor`'s accept job
        // broadcasts the equivalent `TMStatusChange` on every round.
        if !consensus_started {
            if let Some(closed) = root.closed_ledger() {
                use overlay::Overlay;
                let hdr = closed.header();
                let status =
                    overlay::ProtocolMessage::new(overlay::ProtocolPayload::StatusChange(
                        overlay::message::wire::TmStatusChange {
                            new_status: Some(1),
                            new_event: Some(1),
                            ledger_seq: Some(hdr.seq),
                            ledger_hash: Some(hdr.hash.as_uint256().data().to_vec()),
                            ledger_hash_previous: Some(
                                hdr.parent_hash.as_uint256().data().to_vec(),
                            ),
                            network_time: None,
                            first_seq: Some(1),
                            last_seq: Some(hdr.seq),
                        },
                    ));
                overlay_rt.overlay().broadcast(&status);
            }
        }

        // Before starting consensus, acquire the network's validated ledger.
        // This path only matters for nodes JOINING an existing network
        // (is_genesis == false); genesis nodes already started above.
        if !consensus_started {
            use overlay::Overlay;
            let peers = overlay_rt.overlay().active_peers();
            // Start even without peers like rippled
            let any_ahead = peers.iter().find(|p| {
                let h = p.closed_ledger_hash();
                if h.is_zero() {
                    return false;
                }
                if let Some(lm_rt) = root.ledger_master_runtime() {
                    lm_rt
                        .ledger_master()
                        .get_ledger_by_hash(basics::sha_map_hash::SHAMapHash::new(h))
                        .is_none()
                } else {
                    false
                }
            });
            if let Some(peer) = any_ahead {
                // Trigger acquisition for that peer's ledger.
                let closed_hash = peer.closed_ledger_hash();
                if let Some(lm_rt) = root.ledger_master_runtime() {
                    let mut pending = lm_rt
                        .pending_consensus_ledger
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
            } else if let Some(closed) = root.closed_ledger() {
                // All peers are at genesis or we already have their
                // ledger. Only start if we actually need a network ledger
                // (joining case) -- genesis nodes are handled above and
                // must not re-enter this path.
                if root.need_network_ledger() {
                    // Drain queued proposals into the consensus engine
                    // BEFORE starting so that startRound's
                    // playback_proposals finds them (matching rippled
                    // where proposals arrive via JobQueue before
                    // startRound runs).
                    let proposals = overlay_rt.overlay().take_proposals();
                    for proposal in &proposals {
                        let now = root.shared_time_keeper().close_time();
                        let peer_close_time =
                            basics::chrono::NetClockTimePoint::new(
                                proposal.message.close_time,
                            );
                        let prop = consensus::ConsensusProposal::new(
                            proposal.previous_ledger,
                            proposal.message.propose_seq,
                            proposal.current_tx_hash,
                            peer_close_time,
                            now,
                            proposal.public_key,
                        );
                        consensus_rt.push_proposal(
                            crate::runtime::component_runtime::PendingProposal {
                                now,
                                public_key: proposal.public_key,
                                signature: proposal.message.signature.clone(),
                                suppression_id: proposal.suppression,
                                proposal: prop,
                            },
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
                        overlay_rt.overlay().queued_inbound().clear_transaction_router();
                        last_round_ledger_id = Some(*closed.header().hash.as_uint256());
                        last_timer_tick = std::time::Instant::now();
                    }
                }
            }

            // Validations are now processed exclusively by the consensus event-loop
            // thread via the validation-forwarder. No processing here.

            // If acquisition completed, closed_ledger updated -> start consensus
            if !consensus_started {
                if let Some(closed) = root.closed_ledger() {
                    if closed.header().seq > 1 && network_ops_rt.maybe_begin_consensus_from_validated(
                        consensus_rt.as_ref(),
                        Arc::clone(&closed),
                    ) {
                        tracing::info!(target: "consensus",
                            seq = closed.header().seq,
                            "Consensus started from acquired network ledger"
                        );
                        consensus_started = true;
                        last_round_ledger_id = Some(*closed.header().hash.as_uint256());
                        last_timer_tick = std::time::Instant::now();
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

        // Route incoming TmLedgerData responses with type liTS_CANDIDATE (3)
        // to InboundTransactions::got_data, matching rippled's
        // PeerImp::onMessage(TMLedgerData) calling
        // app_.getInboundTransactions().gotData(ledgerHash, peer, m).
        // Without this, TransactionAcquire objects started by
        // acquire_tx_set (when a peer proposes a tx-set hash we don't
        // have locally) never receive their response data, so the
        // consensus dispute resolution mechanism can never compare
        // differing transaction sets and every node closes with only
        // its own locally-submitted transactions.
        {
            use overlay::Overlay;
            let ledger_data_msgs = overlay_rt.overlay().take_ledger_data();
            for msg in ledger_data_msgs {
                if msg.message.r#type == 3 {
                    // liTS_CANDIDATE response: route to InboundTransactions
                    let nodes_received = msg.message.nodes.len();
                    tracing::info!(target: "consensus",
                        peer_id = msg.peer_id,
                        nodes_received,
                        "routing liTS_CANDIDATE response to InboundTransactions"
                    );
                    let hash = msg.message.ledger_hash.as_slice();
                    if let Some(hash) = Uint256::from_slice(hash) {
                        let peer = overlay_rt
                            .overlay()
                            .find_peer_by_short_id(msg.peer_id);
                        let mut guard = root
                            .inbound_transactions()
                            .lock()
                            .expect("inbound_transactions mutex");
                        let status = guard.got_data(hash, peer.clone(), &msg.message);
                        // Check if the acquisition completed after feeding data.
                        // With fat_leaves=true serving, the full tree arrives in
                        // one response and completes immediately (rxrpl-style bypass).
                        if let Some(acquire) = guard.acquire(hash) {
                            if acquire.is_complete() {
                                tracing::info!(target: "consensus",
                                    %hash,
                                    nodes_received,
                                    "tx-set acquisition completed in single response (rxrpl-style)"
                                );
                                let set = Arc::new(acquire.map().clone());
                                guard.give_set(hash, set, true);
                            } else {
                                tracing::debug!(target: "consensus",
                                    %hash,
                                    nodes_received,
                                    has_root = acquire.has_root(),
                                    "tx-set acquisition NOT complete after response, will need more round-trips"
                                );
                            }
                        }
                        drop(guard);
                    }
                }
                // Non-liTS_CANDIDATE TmLedgerData responses (types 0,1,2)
                // are ledger acquisition responses already handled via the
                // SharedInboundLedgers pipeline wired separately.
            }
        }

        // Drain and feed peer proposals into the consensus engine.
        // Always drain here — proposals go into the pending queue which
        // timer_tick (driven by either bootstrap or main.rs) consumes.
        if consensus_started {
            let proposals = overlay_rt.overlay().take_proposals();
            for proposal in &proposals {
                // If peers are proposing on a ledger we don't have, trigger
                // acquisition so we can switch to their chain.
                if let Some(lm_rt) = root.ledger_master_runtime() {
                    let lm = lm_rt.ledger_master();
                    if lm
                        .get_ledger_by_hash(basics::sha_map_hash::SHAMapHash::new(
                            proposal.previous_ledger,
                        ))
                        .is_none()
                    {
                        let mut pending = lm_rt
                            .pending_consensus_ledger
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
                let now = root.shared_time_keeper().close_time();
                let peer_close_time =
                    basics::chrono::NetClockTimePoint::new(proposal.message.close_time);
                let prop = consensus::ConsensusProposal::new(
                    proposal.previous_ledger,
                    proposal.message.propose_seq,
                    proposal.current_tx_hash,
                    peer_close_time,
                    now,
                    proposal.public_key,
                );
                consensus_rt.push_proposal(crate::runtime::component_runtime::PendingProposal {
                    now,
                    public_key: proposal.public_key,
                    signature: proposal.message.signature.clone(),
                    suppression_id: proposal.suppression,
                    proposal: prop,
                });
                // Wake loop immediately so proposal is processed and
                // shouldCloseLedger can check proposersClosed ASAP.
                root.notify_tx_pending();
            }

            // Matches the reference's `PeerImp::onMessage(TMProposeSet)`
            // calling `app_.getOPs().processTrustedProposal(peerPos)` ->
            // `consensus_.peerProposal(...)` SYNCHRONOUSLY, directly from
            // the network I/O thread, the instant a proposal arrives --
            // completely independent of `Consensus::timerEntry`'s own 1s
            // cadence (`ledgerGRANULARITY`). Without this call, proposals
            // pushed into `pending_proposals` above only ever got drained
            // inside `handle_consensus_timer` (gated by the 1s
            // `maybe_tick_consensus!` cooldown below), meaning a proposal
            // that arrived over the wire in milliseconds could sit unread
            // by the algorithm for up to a full second. Confirmed via a
            // live cluster test: nodes reached `update_our_positions` with
            // `curr_peer_positions` empty on nearly every round despite
            // peers continuously sending proposals, causing each node to
            // pick its own close time independently and permanently fork
            // from genesis+1 onward. `drain_proposals` (unlike
            // `handle_consensus_timer`) only feeds `peer_proposal` and
            // does NOT run the heavier `phase_open`/`phase_establish`
            // state machine, so calling it every ~50ms iteration is cheap
            // and matches the reference's real latency characteristics.
            network_ops_rt.drain_proposals(consensus_rt.as_ref());

            // Consume pending_consensus_ledger and trigger InboundLedger
            // acquisition. This is the bridge between fork detection (which
            // sets pending_consensus_ledger) and the actual P2P fetch of the
            // ledger the network is on.
            if let Some(lm_rt) = root.ledger_master_runtime() {
                let pending = lm_rt.take_pending_consensus_ledger();
                if let Some(hash) = pending {
                    shared_inbound.acquire(hash, 0);
                }
            }
        }

        // Batch apply is handled by the dedicated tx-batch-apply thread.
        // By the time the 1s consensus timer fires below, that thread has
        // already applied ALL relay transactions from the past ~995ms.

        maybe_tick_consensus!();

        // Process fetch pack / get-object-by-hash messages (responses AND
        // requests). Responses (fetch-pack data arriving from a peer we
        // asked) are cheap cache inserts and stay inline so
        // `signal_fetch_pack_ready` fires promptly. Requests (a peer
        // asking US to walk our own SHAMap/NodeStore) dispatch onto the
        // `JtLedgerReq` job queue -- matching the reference's
        // `PeerImp::onMessage(TMGetObjectByHash)` calling
        // `app_.getJobQueue().addJob(JtLedgerReq, "RcvGetObjByHash", ...)`
        // -- so this loop's thread never blocks on another peer's
        // catch-up request before it can reach `drain_proposals` again.
        {
            let messages = overlay_rt.overlay().take_get_objects();
            for msg_envelope in messages {
                let msg = &msg_envelope.message;

                if !msg.query {
                    // Response: store objects in FetchPackCache (matching rippled gotFetchPack).
                    if msg.r#type != 6 {
                        continue;
                    }
                    if let Some(lm_rt) = root.ledger_master_runtime() {
                        let lm = lm_rt.ledger_master();
                        let mut stored = 0;
                        for obj in &msg.objects {
                            if let (Some(hash_bytes), Some(data)) = (&obj.hash, &obj.data) {
                                if let Some(hash) = Uint256::from_slice(hash_bytes) {
                                    lm.fetch_pack_cache().add_fetch_pack(hash, data.clone());
                                    stored += 1;
                                }
                            }
                        }
                        if stored > 0 {
                            tracing::info!(target: "consensus",
                                stored,
                                "Fetch pack received and cached"
                            );
                            root.signal_fetch_pack_ready();
                        }
                    }
                } else if msg.r#type
                    == overlay::message::wire::tm_get_object_by_hash::ObjectType::OtFetchPack as i32
                {
                    // Request: peer asks us for a fetch pack (matching rippled doFetchPack/makeFetchPack).
                    let job_root = root.clone();
                    let job_overlay_rt = Arc::clone(&overlay_rt);
                    root.job_queue().add_job(
                        crate::job::job_types::JobType::JtLedgerReq,
                        "RcvGetObjByHash",
                        move || {
                            serve_fetch_pack_request(&job_root, &job_overlay_rt, &msg_envelope);
                        },
                    );
                } else {
                    // Generic GetObjectByHash query (state/tx/ledger nodes).
                    let job_root = root.clone();
                    let job_overlay_rt = Arc::clone(&overlay_rt);
                    root.job_queue().add_job(
                        crate::job::job_types::JobType::JtLedgerReq,
                        "RcvGetObjByHash",
                        move || {
                            serve_get_object_by_hash_request(&job_root, &job_overlay_rt, &msg_envelope);
                        },
                    );
                }
            }
        }
        maybe_tick_consensus!();

        // storeLedger: drain completed InboundLedger results into LedgerHistory
        // and forward to the consensus event loop for checkAccept promotion.
        if let Some(lm_rt) = root.ledger_master_runtime() {
            let rx_guard = lm_rt
                .completed_ledgers_rx
                .lock()
                .expect("completed_ledgers_rx");
            if let Some(rx) = rx_guard.as_ref() {
                while let Ok(ledger) = rx.try_recv() {
                    let inserted = lm_rt
                        .ledger_master()
                        .ledger_history()
                        .insert(std::sync::Arc::clone(&ledger), true);
                    if !inserted {
                        tracing::warn!(
                            target: "consensus",
                            seq = ledger.header().seq,
                            hash = %ledger.header().hash,
                            immutable = ledger.is_immutable(),
                            "Rejected completed ledger insert from completed_ledgers_rx"
                        );
                    }
                    let _ = event_tx.send(
                        crate::consensus::driver::ConsensusEvent::LedgerDone(
                            std::sync::Arc::clone(&ledger),
                        ),
                    );
                }
            }
        }
        while let Ok(ledger) = shared_completed_rx.try_recv() {
            if let Some(lm_rt) = root.ledger_master_runtime() {
                let lm = lm_rt.ledger_master();
                let inserted = lm
                    .ledger_history()
                    .insert(std::sync::Arc::clone(&ledger), true);
                if !inserted {
                    tracing::debug!(
                        target: "consensus",
                        seq = ledger.header().seq,
                        hash = %ledger.header().hash,
                        "Ledger already in history (shared_completed_rx dedup)"
                    );
                }
                // Always mark as complete — whether newly inserted or already
                // present. The ledger was successfully acquired and verified.
                let ledger_seq = ledger.header().seq;
                if ledger_seq > 0 {
                    lm.mark_ledger_complete(ledger_seq);
                }
            }
            let _ = event_tx.send(
                crate::consensus::driver::ConsensusEvent::LedgerDone(ledger),
            );
        }

        // checkAccept + tryAdvance burst catch-up (matching rippled LedgerMaster.cpp):
        // 1. First check if the closed ledger can be promoted to validated.
        // 2. Then loop through consecutive ledgers in LedgerHistory, promoting
        //    each one that has sufficient validations (burst catch-up).
        // This allows validating N+1, N+2, ... N+50 in a single tick if all
        // intermediate ledgers are available in history with quorum validations.
        // Reads `root.closed_ledger()` for the "which ledger just closed"
        // check (the single source of truth), but keeps using
        // `lm.valid_ledger_seq()`/`lm.ledger_history()`/
        // `lm.set_valid_ledger_no_sweep(...)` for the VALIDATED-ledger
        // burst-advance bookkeeping -- that is legitimately
        // `ledger::LedgerMaster`'s own internal state (distinct from "the
        // closed ledger"), not duplicated anywhere else, so it is not part
        // of the dual-tracker bug this rewrite fixes.
        if let Some(lm_rt) = root.ledger_master_runtime() {
            let lm = lm_rt.ledger_master();
            let quorum = root.validators().quorum();

            // -----------------------------------------------------------
            // switchLastClosedLedger:
            // When need_network_ledger is true, we're joining an existing
            // network. Instead of requiring full quorum (which requires our
            // OWN validation — impossible until we're on the same chain),
            // switch to the network's chain when we have an acquired ledger
            // that ANY trusted peer has validated. This matches rippled's
            // When need_network_ledger is true, adopts the network chain when a
            // joining node adopts the network's preferred LCL based on peer
            // consensus, not its own validation count.
            // -----------------------------------------------------------
            if root.need_network_ledger() {
                // Find the best acquired ledger: check recent entries in
                // ledger_history that have at least 1 trusted peer validation
                // and differ from our current closed ledger.
                let our_closed_hash = root.closed_ledger()
                    .map(|l| *l.header().hash.as_uint256())
                    .unwrap_or_default();

                // Check the latest acquired ledger from shared_completed_rx
                // which was just inserted into history above. Walk a range
                // of recent sequences looking for peer-validated ones.
                let mut best_validated: Option<Arc<ledger::Ledger>> = None;
                // Try getting any ledger from history that peers validated.
                // Use the ledger_history's by-hash cache since by-seq may
                // not have all entries.
                let acquired_hashes: Vec<_> = {
                    use overlay::Overlay;
                    let peers = overlay_rt.overlay().active_peers();
                    peers.iter()
                        .map(|p| p.closed_ledger_hash())
                        .filter(|h| !h.is_zero() && *h != our_closed_hash)
                        .collect()
                };
                for hash in &acquired_hashes {
                    let candidate = lm.ledger_history().get_cached_ledger_by_hash(
                        basics::sha_map_hash::SHAMapHash::new(*hash)
                    );
                    if let Some(candidate) = candidate {
                        let candidate_hash = *candidate.header().hash.as_uint256();
                        let val_count = root.validations().num_trusted_for_ledger(candidate_hash);
                        if val_count > 0 {
                            if best_validated.as_ref().map_or(true, |b| candidate.header().seq > b.header().seq) {
                                best_validated = Some(candidate);
                            }
                        }
                    }
                }

                if let Some(network_ledger) = best_validated {
                    // Rippled parity: only switch when the ledger is FULLY
                    // complete (state map + tx map downloaded, not just header).
                    // InboundLedgers::acquire() returns nullptr until isComplete().
                    // Without this check, we'd switch to a ledger whose state
                    // can't be traversed, causing all account queries to fail.
                    let state_complete = !network_ledger.state_map().is_synching();
                    let tx_complete = network_ledger.header().tx_hash.is_zero()
                        || !network_ledger.tx_map().is_synching();

                    if !state_complete || !tx_complete {
                        tracing::debug!(target: "consensus",
                            seq = network_ledger.header().seq,
                            state_complete, tx_complete,
                            "switchLastClosedLedger: waiting for full state download"
                        );
                    } else {
                    let new_seq = network_ledger.header().seq;
                    let new_hash = *network_ledger.header().hash.as_uint256();
                    let val_count = root.validations().num_trusted_for_ledger(new_hash);
                    tracing::info!(target: "consensus",
                        new_seq, %new_hash, val_count,
                        "switchLastClosedLedger: adopting network chain"
                    );

                    // Promote to validated
                    let mut l = (*network_ledger).clone();
                    l.set_validated();
                    let validated = Arc::new(l);
                    lm.set_valid_ledger_no_sweep(Arc::clone(&validated), None, None);
                    lm.mark_ledger_complete(validated.header().seq);
                    root.note_validated_ledger_for_sync(Arc::clone(&validated));

                    // Switch closed ledger to the network's chain
                    root.on_closed_ledger(Arc::clone(&validated));
                    root.set_need_network_ledger(false);

                    // Restart consensus from the new LCL (matches rippled's
                    // switchLastClosedLedger → beginConsensus flow)
                    let now = root.shared_time_keeper().close_time();
                    let prev_cx = crate::consensus_ledger_from_ledger(&validated);
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("switch lcl consensus runtime");
                    rt.block_on(async {
                        consensus_rt.start_round(now, new_hash, prev_cx).await;
                    });
                    last_round_ledger_id = Some(new_hash);
                    tracing::info!(target: "consensus",
                        new_seq, %new_hash,
                        "Consensus restarted on network chain (switchLastClosedLedger)"
                    );
                    } // else (state_complete)
                }
            }

            // checkAccept: promote closed ledger if it has quorum
            if let Some(closed) = root.closed_ledger() {
                let closed_seq = closed.header().seq;
                if closed_seq > lm.valid_ledger_seq() {
                    let closed_hash = *closed.header().hash.as_uint256();
                    let val_count = root.validations().num_trusted_for_ledger(closed_hash);
                    if val_count >= quorum {
                        let mut l = (*closed).clone();
                        l.set_validated();
                        let validated = std::sync::Arc::new(l);
                        lm.set_valid_ledger_no_sweep(std::sync::Arc::clone(&validated), None, None);
                        root.note_validated_ledger_for_sync(std::sync::Arc::clone(&validated));
                        lm.mark_ledger_complete(validated.header().seq);
                        root.set_need_network_ledger(false);
                        tracing::info!(target: "consensus",
                            seq = closed_seq, val_count, quorum,
                            "Validated ledger advanced (--start mode)"
                        );
                    }
                }
            }

            // tryAdvance: burst through consecutive ledgers in history
            // (rippled doAdvance/findNewLedgersToPublish equivalent)
            let mut advanced = 0u32;
            loop {
                let next_seq = lm.valid_ledger_seq() + 1;
                // Look up next ledger in history by sequence
                let next_ledger = lm.ledger_history().get_cached_ledger_by_seq(next_seq);
                let Some(candidate) = next_ledger else {
                    break;
                };
                let candidate_hash = *candidate.header().hash.as_uint256();
                let val_count = root.validations().num_trusted_for_ledger(candidate_hash);
                if val_count < quorum {
                    break;
                }
                // Promote to validated
                let mut l = (*candidate).clone();
                l.set_validated();
                let validated = std::sync::Arc::new(l);
                lm.set_valid_ledger_no_sweep(std::sync::Arc::clone(&validated), None, None);
                root.note_validated_ledger_for_sync(std::sync::Arc::clone(&validated));
                lm.mark_ledger_complete(validated.header().seq);
                root.set_need_network_ledger(false);
                advanced += 1;
            }
            if advanced > 0 {
                tracing::info!(target: "consensus",
                    advanced,
                    new_valid_seq = lm.valid_ledger_seq(),
                    quorum,
                    "tryAdvance burst: validated consecutive ledgers from history"
                );
            }

            // Update complete_ledgers display in StatusRpcState.
            // Without this, server_info falls back to "seq-seq" format.
            let complete_range = lm.complete_ledgers();
            let range_str = complete_range.to_string();
            if !range_str.is_empty() {
                root.set_status_rpc_complete_ledgers(Some(range_str));
            }

            // Operating mode promotion: Connected → Full
            // Matches rippled's endConsensus: promote to Full when:
            // 1. Mode is Connected (not Disconnected)
            // 2. Validated ledger parent is in complete_ledgers range
            // 3. Close time is recent (handled by normalize_operating_mode_for_validated_age)
            {
                use crate::network::network_ops::NetworkOpsOperatingMode;
                let current_mode = root.network_ops_state().operating_mode();
                if current_mode == NetworkOpsOperatingMode::Connected {
                    let valid_seq = lm.valid_ledger_seq();
                    if valid_seq > 1 && lm.have_ledger(valid_seq - 1) {
                        root.set_network_ops_operating_mode(NetworkOpsOperatingMode::Full);
                    }
                }
            }

            // Gap-fill: only request fetch packs during initial catchup.
            // Once we have a validated ledger, consensus builds new ledgers
            // directly — no need to fetch from peers.
            if advanced == 0 && lm.valid_ledger_seq() == 0 {
                use overlay::Overlay;
                let our_closed_hash = root
                    .closed_ledger()
                    .map(|l| *l.header().hash.as_uint256())
                    .unwrap_or(basics::base_uint::Uint256::zero());
                let peers = overlay_rt.overlay().active_peers();
                let in_sync = peers
                    .iter()
                    .any(|p| p.closed_ledger_hash() == our_closed_hash);
                // Only request when out of sync, throttled to once per 30 seconds
                if !in_sync && !peers.is_empty() {
                    static LAST_GAP_FILL: std::sync::atomic::AtomicU64 =
                        std::sync::atomic::AtomicU64::new(0);
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    let last = LAST_GAP_FILL.load(std::sync::atomic::Ordering::Relaxed);
                    if now_ms.saturating_sub(last) >= 30_000 {
                        LAST_GAP_FILL.store(now_ms, std::sync::atomic::Ordering::Relaxed);
                        for p in &peers {
                            let peer_hash = p.closed_ledger_hash();
                            if !peer_hash.is_zero() {
                                let fp_msg = ledger::make_fetch_pack_request(
                                    basics::sha_map_hash::SHAMapHash::new(peer_hash),
                                );
                                let wire = overlay::Message::new(fp_msg, None);
                                p.send(wire);
                                break;
                            }
                        }
                    }
                }
            }
        }

        // ─── History Backfill (rippled doAdvance/fetchForHistory parity) ───
        // After tracking head, backfill missing historical ledgers backwards
        // from the validated seq. This fills the complete_ledgers range so the
        // node eventually reaches 'full' operating mode.
        // Throttled to once per 3 seconds to avoid overwhelming peers.
        if let Some(lm_rt) = root.ledger_master_runtime() {
            let lm = lm_rt.ledger_master();
            let valid_seq = lm.valid_ledger_seq();
            // Only backfill if we have a validated ledger
            if valid_seq > 1 {
                if last_history_tick.elapsed() >= Duration::from_secs(3) {
                    last_history_tick = std::time::Instant::now();

                    // Find the first missing ledger in the range
                    let ledger_history_limit = 512u32; // from [ledger_history] config
                    let earliest_wanted = valid_seq.saturating_sub(ledger_history_limit);
                    let complete = lm.complete_ledgers();

                    // Walk backwards to find the first gap
                    let mut missing_seq = None;
                    for seq in (earliest_wanted..valid_seq).rev() {
                        if seq <= 1 { break; }
                        if !complete.contains(seq) {
                            missing_seq = Some(seq);
                            break;
                        }
                    }

                    if let Some(missing) = missing_seq {
                        // Get the hash for the missing ledger from the next ledger's parent_hash
                        let parent_hash = lm.ledger_history()
                            .get_cached_ledger_by_seq(missing + 1)
                            .map(|l| *l.header().parent_hash.as_uint256());

                        if let Some(hash) = parent_hash {
                            if !hash.is_zero() {
                                let sha_hash = basics::sha_map_hash::SHAMapHash::new(hash);
                                // Check if we already have it in history
                                let already_have = lm.ledger_history()
                                    .get_cached_ledger_by_hash(sha_hash)
                                    .is_some();

                                if !already_have {
                                    // Acquire from peers
                                    shared_inbound.acquire(hash, missing);
                                    tracing::debug!(target: "consensus",
                                        missing, %hash,
                                        "history backfill: acquiring missing ledger"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        // Tick pending TransactionAcquire objects at ~1s cadence (matching
        // rippled's InboundTransactions timer). Must NOT be called every
        // 50ms loop iteration — invoke_on_timer increments timeouts on
        // Tick pending tx-set acquisitions. 500ms cadence balances fast
        // multi-round-trip SHAMap downloads against timeout sensitivity.
        if last_acquire_tick.elapsed() >= Duration::from_millis(500) {
            let mut guard = root
                .inbound_transactions()
                .lock()
                .expect("inbound_transactions mutex");
            guard.tick_pending_acquires();
            last_acquire_tick = std::time::Instant::now();
        }

        // Main loop polls at 50ms for proposal processing and ledger
        // requests. Consensus tick (shouldCloseLedger/on_close) is gated
        // to 1-second inside maybe_tick_consensus. This matches rippled:
        // proposals delivered immediately, timer fires every 1 second.
        std::thread::sleep(Duration::from_millis(50));
    }

    tracing::info!(target: "consensus", "Start-mode consensus event loop stopped");
}

/// Drain queued `TMGetLedger` requests and dispatch each one onto the
/// `JtLedgerReq` job queue, matching the reference's
/// `PeerImp::onMessage(TMGetLedger)` calling
/// `app_.getJobQueue().addJob(JtLedgerReq, "RcvGetLedger", ...)`. The actual
/// SHAMap node walk (`serve_one_get_ledger_request`) runs entirely inside
/// the dispatched job, on a `JobQueue` worker thread -- never on the
/// caller's thread -- so a burst of peer catch-up requests cannot delay
/// whatever the caller needs to do next (in particular, this loop's own
/// `drain_proposals` call later in the same iteration).
fn dispatch_get_ledger_requests(
    root: &crate::ApplicationRoot,
    overlay_rt: &Arc<crate::runtime::overlay_runtime::AppOverlayRuntime>,
) {
    let requests = overlay_rt.overlay().take_get_ledgers();
    if requests.is_empty() {
        return;
    }

    for req in requests {
        if req.message.itype == 3 {
            // liTS_CANDIDATE: serve inline for minimal latency. Tx-set
            // lookups are just a HashMap get + serialize — fast enough to
            // run synchronously, and time-critical for dispute resolution
            // to complete within the consensus round.
            serve_one_get_ledger_request(root, overlay_rt, req);
        } else {
            let job_root = root.clone();
            let job_overlay_rt = Arc::clone(overlay_rt);
            root.job_queue().add_job(
                crate::job::job_types::JobType::JtLedgerReq,
                "RcvGetLedger",
                move || {
                    serve_one_get_ledger_request(&job_root, &job_overlay_rt, req);
                },
            );
        }
    }
}

fn serve_one_get_ledger_request(
    root: &crate::ApplicationRoot,
    overlay_rt: &Arc<crate::runtime::overlay_runtime::AppOverlayRuntime>,
    req: overlay::PeerMessage<overlay::TmGetLedger>,
) {
    use overlay::Overlay;

    let Some(hash_bytes) = req.message.ledger_hash.as_deref() else {
        return;
    };
    let Some(hash) = Uint256::from_slice(hash_bytes) else {
        return;
    };

    let itype = req.message.itype;
    let mut nodes: Vec<overlay::message::wire::TmLedgerNode> = Vec::new();

    // liTS_CANDIDATE (3) uses InboundTransactions, not LedgerMaster.
    // Handle it before the ledger lookup which would early-return for
    // tx-set hashes that aren't ledger hashes.
    if itype == 3 {
        let mut guard = root
            .inbound_transactions()
            .lock()
            .expect("inbound_transactions mutex");
        let set = guard.get_set(hash, false);
        if set.is_none() {
            // Log both the requested hash AND every stored hash for direct comparison
            let stored: Vec<Uint256> = guard.stored_hashes();
            let match_found = stored.iter().any(|h| *h == hash);
            tracing::warn!(target: "consensus",
                requested = %hash,
                stored_count = stored.len(),
                btree_match = match_found,
                "liTS_CANDIDATE: set not found"
            );
            if !stored.is_empty() {
                for (i, h) in stored.iter().enumerate().take(3) {
                    tracing::warn!(target: "consensus",
                        index = i,
                        stored_hash = %h,
                        bytes_match = (h.data() == hash.data()),
                        "liTS_CANDIDATE: stored hash comparison"
                    );
                }
            }
            drop(guard);
            return;
        }
        drop(guard);
        let sync_tree = set.unwrap();
        let mut fetch = |_h: basics::sha_map_hash::SHAMapHash| -> Option<
            basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            >,
        > { None };
        let requested_node_ids = &req.message.node_i_ds;
        if requested_node_ids.is_empty() {
            // No specific nodes requested: nothing to do
            return;
        }
        // Check if this is a root-only request (first request from TransactionAcquire).
        // If so, serve ALL nodes at once for 1-round-trip acquisition (matching rxrpl).
        let is_root_request = requested_node_ids.len() == 1
            && requested_node_ids[0] == shamap::nodes::node_id::SHAMapNodeId::default().get_raw_string();

        if is_root_request {
            // Serve the entire tree in one response: root + all inner + all leaves.
            // depth=8 covers any realistic tx-set (1000 txns fit in depth 4-5).
            // fat_leaves=true ensures leaf nodes (actual transactions) are included,
            // enabling single-round-trip acquisition (rxrpl-style bypass).
            let mut fetch = |_h: basics::sha_map_hash::SHAMapHash| -> Option<
                basics::memory::intrusive_pointer::SharedIntrusive<
                    shamap::nodes::tree_node::SHAMapTreeNode,
                >,
            > { None };
            let root_id = shamap::nodes::node_id::SHAMapNodeId::default();
            let mut data: Vec<(shamap::nodes::node_id::SHAMapNodeId, Vec<u8>)> = Vec::new();
            let _ = sync_tree.get_node_fat(root_id, &mut data, true, 8, &mut fetch);
            tracing::debug!(target: "consensus",
                %hash,
                total_nodes = data.len(),
                "liTS_CANDIDATE: serving full tree (fat_leaves=true, depth=8)"
            );
            for (nid, ndata) in &data {
                nodes.push(overlay::message::wire::TmLedgerNode {
                    nodeid: Some(nid.get_raw_string()),
                    nodedata: ndata.clone(),
                });
                if nodes.len() >= 2048 {
                    break;
                }
            }
        } else {
            for node_id_bytes in requested_node_ids {
                let Some(node_id) =
                    shamap::nodes::node_id::deserialize_shamap_node_id(node_id_bytes)
                else {
                    continue;
                };
                let mut data: Vec<(shamap::nodes::node_id::SHAMapNodeId, Vec<u8>)> = Vec::new();
                if sync_tree
                    .get_node_fat(node_id, &mut data, false, 1, &mut fetch)
                    .is_ok()
                {
                    for (nid, ndata) in &data {
                        nodes.push(overlay::message::wire::TmLedgerNode {
                            nodeid: Some(nid.get_raw_string()),
                            nodedata: ndata.clone(),
                        });
                        if nodes.len() >= 256 {
                            break;
                        }
                    }
                }
                if nodes.len() >= 256 {
                    break;
                }
            }
        }

        if nodes.is_empty() {
            tracing::warn!(target: "consensus", %hash, "liTS_CANDIDATE: serialization produced empty nodes");
            return;
        }

        let response_data = overlay::TmLedgerData {
            ledger_hash: hash.data().to_vec(),
            ledger_seq: 0,
            r#type: 3,
            nodes,
            request_cookie: req.message.request_cookie.map(|c| c as u32),
            error: None,
        };
        tracing::info!(target: "consensus",
            %hash,
            nodes_count = response_data.nodes.len(),
            first_node_data_len = response_data.nodes.first().map(|n| n.nodedata.len()).unwrap_or(0),
            "liTS_CANDIDATE: sending response (as type 3)"
        );
        let response = overlay::ProtocolMessage::new(overlay::ProtocolPayload::LedgerData(response_data));
        let message = overlay::Message::new(response, None);
        if let Some(peer) = overlay_rt.overlay().find_peer_by_short_id(req.peer_id) {
            peer.send(message);
        }
        return;
    }

    let Some(lm_rt) = root.ledger_master_runtime() else {
        return;
    };
    let lm = lm_rt.ledger_master();

    let Some(ledger) = lm.get_ledger_by_hash(basics::sha_map_hash::SHAMapHash::new(hash)) else {
        return;
    };

    match itype {
        0 => {
            // li_BASE: header + state root + tx root (matching rippled sendLedgerBase)
            let header_data = protocol::serialize_ledger_header(&ledger.header(), false);
            nodes.push(overlay::message::wire::TmLedgerNode {
                nodeid: None,
                nodedata: header_data,
            });
            // State map root
            if !ledger.header().account_hash.is_zero() {
                if let Ok(root_data) = ledger.state_map().serialize_root() {
                    nodes.push(overlay::message::wire::TmLedgerNode {
                        nodeid: None,
                        nodedata: root_data,
                    });
                }
            }
            // Tx map root
            if !ledger.header().tx_hash.is_zero() {
                if let Ok(root_data) = ledger.tx_map().serialize_root() {
                    nodes.push(overlay::message::wire::TmLedgerNode {
                        nodeid: None,
                        nodedata: root_data,
                    });
                }
            }
        }
        1 | 2 => {
            // liTX_NODE (1) or liAS_NODE (2): serve requested SHAMap nodes
            let map = if itype == 1 {
                ledger.tx_map()
            } else {
                ledger.state_map()
            };
            let fat_leaves = itype == 1; // fat for TX, not for AS
            let depth = req.message.query_depth.unwrap_or(1);

            for node_id_bytes in &req.message.node_i_ds {
                let Some(node_id) =
                    shamap::nodes::node_id::deserialize_shamap_node_id(node_id_bytes)
                else {
                    continue;
                };
                let mut data: Vec<(shamap::nodes::node_id::SHAMapNodeId, Vec<u8>)> = Vec::new();
                let mut fetch = |_h: basics::sha_map_hash::SHAMapHash| -> Option<
                    basics::memory::intrusive_pointer::SharedIntrusive<
                        shamap::nodes::tree_node::SHAMapTreeNode,
                    >,
                > { None };
                if map
                    .get_node_fat(node_id, &mut data, fat_leaves, depth, &mut fetch)
                    .is_ok()
                {
                    for (nid, ndata) in &data {
                        nodes.push(overlay::message::wire::TmLedgerNode {
                            nodeid: Some(nid.get_raw_string()),
                            nodedata: ndata.clone(),
                        });
                        if nodes.len() >= 256 {
                            break;
                        }
                    }
                }
                if nodes.len() >= 256 {
                    break;
                }
            }
        }
        _ => return,
    }

    if nodes.is_empty() {
        return;
    }

    let response = overlay::ProtocolMessage::new(overlay::ProtocolPayload::LedgerData(
        overlay::TmLedgerData {
            ledger_hash: hash.data().to_vec(),
            ledger_seq: ledger.header().seq,
            r#type: itype,
            nodes,
            request_cookie: req.message.request_cookie.map(|c| c as u32),
            error: None,
        },
    ));
    let message = overlay::Message::new(response, None);
    if let Some(peer) = overlay_rt.overlay().find_peer_by_short_id(req.peer_id) {
        peer.send(message);
    }
}

/// Serve a fetch pack request from a peer (matching rippled doFetchPack/makeFetchPack).
///
/// Given a `TMGetObjectByHash` with `query=true, type=otFETCH_PACK`, we:
/// 1. Look up the ledger with the requested hash (`have`).
/// 2. Look up its parent ledger (`want`).
/// 3. Walk the state map diff (nodes in `have` not in `want`) using `visit_differences`.
/// 4. Serialize each differing node with its hash and send as response.
/// Cap at 512 objects per response, matching rippled.
fn serve_fetch_pack_request(
    root: &crate::ApplicationRoot,
    overlay_rt: &Arc<crate::runtime::overlay_runtime::AppOverlayRuntime>,
    req: &overlay::PeerMessage<overlay::TmGetObjectByHash>,
) {
    use overlay::Overlay;

    let Some(ledger_hash_bytes) = req.message.ledger_hash.as_deref() else {
        return;
    };
    let Some(ledger_hash) = Uint256::from_slice(ledger_hash_bytes) else {
        return;
    };
    let Some(lm_rt) = root.ledger_master_runtime() else {
        return;
    };
    let lm = lm_rt.ledger_master();

    // Get the ledger the peer specified ("have" in rippled terms).
    let Some(have) = lm.get_ledger_by_hash(basics::sha_map_hash::SHAMapHash::new(ledger_hash))
    else {
        return;
    };

    // Get its parent ("want" in rippled terms — the ledger the peer needs to catch up to).
    let parent_hash = *have.header().parent_hash.as_uint256();
    if parent_hash == Uint256::zero() {
        return;
    }
    let Some(want) = lm.get_ledger_by_hash(basics::sha_map_hash::SHAMapHash::new(parent_hash))
    else {
        return;
    };

    // Diff state maps: find nodes in `have` that are missing from `want`.
    // This matches rippled's populateFetchPack(have->stateMap(), &want->stateMap(), 16384, ...).
    let have_root = have.state_map().root();
    let want_root = want.state_map().root();

    let mut objects: Vec<overlay::message::wire::TmIndexedObject> = Vec::new();
    let have_seq = have.header().seq;
    let mut no_op_fetch = |_h: basics::sha_map_hash::SHAMapHash| -> Option<
        basics::memory::intrusive_pointer::SharedIntrusive<shamap::tree_node::SHAMapTreeNode>,
    > { None };
    let mut no_op_fetch2 = |_h: basics::sha_map_hash::SHAMapHash| -> Option<
        basics::memory::intrusive_pointer::SharedIntrusive<shamap::tree_node::SHAMapTreeNode>,
    > { None };

    let _ = shamap::difference::visit_differences(
        &have_root,
        Some(&want_root),
        have.state_map().backed(),
        &mut no_op_fetch,
        want.state_map().backed(),
        &mut no_op_fetch2,
        &mut |node: &basics::memory::intrusive_pointer::SharedIntrusive<
            shamap::tree_node::SHAMapTreeNode,
        >| {
            if objects.len() >= 512 {
                return false; // stop iteration
            }
            let hash = node.get_hash();
            if let Ok(data) = node.serialize_with_prefix() {
                objects.push(overlay::message::wire::TmIndexedObject {
                    hash: Some(hash.as_uint256().data().to_vec()),
                    node_id: None,
                    index: None,
                    data: Some(data),
                    ledger_seq: Some(have_seq),
                });
            }
            objects.len() < 512
        },
    );

    if objects.is_empty() {
        return;
    }

    tracing::info!(target: "consensus",
        objects = objects.len(),
        seq = have_seq,
        "Serving fetch pack to peer"
    );

    let reply = overlay::TmGetObjectByHash {
        r#type: overlay::message::wire::tm_get_object_by_hash::ObjectType::OtFetchPack as i32,
        query: false,
        ledger_hash: Some(ledger_hash_bytes.to_vec()),
        fat: None,
        objects,
    };

    let response = overlay::ProtocolMessage::new(overlay::ProtocolPayload::GetObjects(reply));
    let message = overlay::Message::new(response, None);
    if let Some(peer) = overlay_rt.overlay().find_peer_by_short_id(req.peer_id) {
        peer.send(message);
    }
}

// --- GetObjectByHash rate limiting constants (matching rippled Tuning.h) ---

/// Hard ceiling: reject requests asking for more than this many objects.
const HARD_MAX_REPLY_NODES: usize = 12_288;

/// First N objects per request are free (no cost charged).
const FREE_OBJECTS_PER_REQUEST: u32 = 16;

/// Cost per billable lookup that hits the cache/node store.
const COST_PER_LOOKUP_HIT: u32 = 1;

/// Cost per billable lookup that misses (not found in node store).
const COST_PER_LOOKUP_MISS: u32 = 8;

/// Size band boundary: requests with ≤64 objects are "small".
const BAND_SMALL_MAX: usize = 64;

/// Size band boundary: requests with ≤1024 objects are "medium".
const BAND_MEDIUM_MAX: usize = 1024;

/// Surcharge for small requests (none).
const COST_BAND_SMALL: u32 = 0;

/// Surcharge for medium-sized requests.
const COST_BAND_MEDIUM: u32 = 100;

/// Surcharge for large requests (>1024 objects).
const COST_BAND_LARGE: u32 = 1000;

/// If the computed cost exceeds this threshold, charge and warn about the peer.
const DROP_THRESHOLD: u32 = 25_000;

/// Serve a generic GetObjectByHash query from a peer (matching rippled processGetObjectByHash).
///
/// Looks up each requested hash in the node store, tracks hits/misses, applies
/// differential pricing, and sends a reply. Oversized requests are rejected
/// immediately. Excessively costly requests charge the peer.
fn serve_get_object_by_hash_request(
    root: &crate::ApplicationRoot,
    overlay_rt: &Arc<crate::runtime::overlay_runtime::AppOverlayRuntime>,
    req: &overlay::PeerMessage<overlay::TmGetObjectByHash>,
) {
    use overlay::Overlay;

    let msg = &req.message;
    let requested = msg.objects.len();

    // Hard limit: reject oversized requests before touching the node store.
    if requested > HARD_MAX_REPLY_NODES {
        tracing::warn!(target: "overlay",
            peer_id = req.peer_id,
            requested,
            limit = HARD_MAX_REPLY_NODES,
            "GetObjectByHash: oversized request rejected"
        );
        return;
    }

    let Some(node_store) = root.node_store().as_ref() else {
        return;
    };

    let mut reply_objects: Vec<overlay::message::wire::TmIndexedObject> = Vec::new();
    let mut hits: u32 = 0;
    let mut misses: u32 = 0;

    let iter_limit = requested.min(HARD_MAX_REPLY_NODES);
    for obj in msg.objects.iter().take(iter_limit) {
        let Some(hash_bytes) = obj.hash.as_deref() else {
            misses += 1;
            continue;
        };
        let Some(hash) = Uint256::from_slice(hash_bytes) else {
            misses += 1;
            continue;
        };

        let ledger_seq = obj.ledger_seq.unwrap_or(0);
        let fetched = match node_store {
            crate::SHAMapStoreNodeStore::Single(database) => {
                database.fetch_node_object(&hash, ledger_seq, FetchType::Synchronous, false)
            }
            crate::SHAMapStoreNodeStore::Rotating(database) => {
                database.fetch_node_object(&hash, ledger_seq, FetchType::Synchronous, false)
            }
        };

        if let Some(node_object) = fetched {
            hits += 1;
            reply_objects.push(overlay::message::wire::TmIndexedObject {
                hash: Some(hash.data().to_vec()),
                node_id: None,
                index: obj.node_id.clone(),
                data: Some(node_object.data().clone()),
                ledger_seq: obj.ledger_seq,
            });
        } else {
            misses += 1;
        }
    }

    // Compute differential cost (matching rippled computeGetObjectByHashFee).
    let billable = (requested as u32).saturating_sub(FREE_OBJECTS_PER_REQUEST);
    let billable_misses = misses.min(billable);
    let billable_hits = billable.saturating_sub(billable_misses);

    let size_band = if requested > BAND_MEDIUM_MAX {
        COST_BAND_LARGE
    } else if requested > BAND_SMALL_MAX {
        COST_BAND_MEDIUM
    } else {
        COST_BAND_SMALL
    };

    let cost =
        billable_hits * COST_PER_LOOKUP_HIT + billable_misses * COST_PER_LOOKUP_MISS + size_band;

    if cost > DROP_THRESHOLD {
        tracing::warn!(target: "overlay",
            peer_id = req.peer_id,
            requested,
            hits,
            misses,
            cost,
            threshold = DROP_THRESHOLD,
            "GetObjectByHash: cost exceeds drop threshold, charging peer"
        );
        if let Some(peer) = overlay_rt.overlay().find_peer_by_short_id(req.peer_id) {
            peer.charge(
                resource::Charge::new(cost as i32, "GetObjectByHash excessive cost"),
                "GetObjectByHash cost exceeded drop threshold".to_owned(),
            );
            overlay_rt.overlay().inc_peer_disconnect_charges();
        }
        return;
    }

    // Charge the peer with the computed cost.
    if cost > 0 {
        if let Some(peer) = overlay_rt.overlay().find_peer_by_short_id(req.peer_id) {
            peer.charge(
                resource::Charge::new(cost as i32, "GetObjectByHash differential"),
                "processed get object by hash request".to_owned(),
            );
        }
    }

    // Send reply only if we found at least one object.
    if reply_objects.is_empty() {
        return;
    }

    tracing::trace!(target: "overlay",
        peer_id = req.peer_id,
        found = reply_objects.len(),
        requested,
        cost,
        "GetObjectByHash: serving reply"
    );

    let reply = overlay::TmGetObjectByHash {
        r#type: msg.r#type,
        query: false,
        ledger_hash: msg.ledger_hash.clone(),
        fat: msg.fat,
        objects: reply_objects,
    };

    let response = overlay::ProtocolMessage::new(overlay::ProtocolPayload::GetObjects(reply));
    let message = overlay::Message::new(response, None);
    if let Some(peer) = overlay_rt.overlay().find_peer_by_short_id(req.peer_id) {
        peer.send(message);
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
        StartUpType::Fresh | StartUpType::Normal | StartUpType::Snapshot => {
            seed_startup_ledger_state(root, options, config)
        }
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
            let genesis_amendments = amendments_from_config(config, options.standalone);
            let genesis_config = LedgerConfig {
                fees: ledger::CURRENT_DEFAULT_FEES,
                ..LedgerConfig::default()
            };
            Ledger::create_genesis(backed, &genesis_config, genesis_amendments)
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
    if options.standalone
        || options.start_valid
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

fn amendments_from_config(config: &BasicConfig, standalone: bool) -> Vec<Uint256> {
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
    // Standalone: enable ALL supported amendments (matching rippled standalone).
    // Network mode: only amendments voted DefaultYes.
    if standalone {
        REGISTERED_FEATURES
            .iter()
            .filter(|f| f.supported)
            .map(|f| feature_id(f.name))
            .collect()
    } else {
        REGISTERED_FEATURES
            .iter()
            .filter(|f| f.supported && f.vote == RegisteredFeatureVote::DefaultYes)
            .map(|f| feature_id(f.name))
            .collect()
    }
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

/// Parse the `[transaction_queue]` config section.
/// All fields are optional — unset fields use TxQSetup::default().
fn parse_txq_setup(config: &BasicConfig) -> tx::TxQSetup {
    use tx::TxQSetup;
    let mut setup = TxQSetup::default();

    if !config.exists("transaction_queue") {
        return setup;
    }

    let section_values: Vec<(String, String)> = config
        .section("transaction_queue")
        .values()
        .iter()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(2, '=').collect();
            if parts.len() == 2 {
                Some((parts[0].trim().to_string(), parts[1].trim().to_string()))
            } else {
                None
            }
        })
        .collect();

    for (key, value) in &section_values {
        match key.as_str() {
            "ledgers_in_queue" => {
                if let Ok(v) = value.parse::<usize>() { setup.ledgers_in_queue = v; }
            }
            "minimum_queue_size" => {
                if let Ok(v) = value.parse::<usize>() { setup.queue_size_min = v; }
            }
            "retry_sequence_percent" => {
                if let Ok(v) = value.parse::<u32>() { setup.retry_sequence_percent = v; }
            }
            "minimum_txn_in_ledger" => {
                if let Ok(v) = value.parse::<usize>() { setup.minimum_txn_in_ledger = v; }
            }
            "minimum_txn_in_ledger_standalone" => {
                if let Ok(v) = value.parse::<usize>() { setup.minimum_txn_in_ledger_standalone = v; }
            }
            "target_txn_in_ledger" => {
                if let Ok(v) = value.parse::<usize>() { setup.target_txn_in_ledger = v; }
            }
            "maximum_txn_in_ledger" => {
                if let Ok(v) = value.parse::<usize>() { setup.maximum_txn_in_ledger = Some(v); }
            }
            "normal_consensus_increase_percent" => {
                if let Ok(v) = value.parse::<u32>() {
                    setup.normal_consensus_increase_percent = v.clamp(0, 1000);
                }
            }
            "slow_consensus_decrease_percent" => {
                if let Ok(v) = value.parse::<u32>() {
                    setup.slow_consensus_decrease_percent = v.clamp(0, 100);
                }
            }
            "maximum_txn_per_account" => {
                if let Ok(v) = value.parse::<u32>() { setup.maximum_txn_per_account = v; }
            }
            "minimum_last_ledger_buffer" => {
                if let Ok(v) = value.parse::<u32>() { setup.minimum_last_ledger_buffer = v; }
            }
            _ => {
                tracing::warn!(target: "bootstrap", key, "Unknown [transaction_queue] config key");
            }
        }
    }

    // Validation: maximum must not be less than minimum
    if let Some(max) = setup.maximum_txn_in_ledger {
        if max < setup.minimum_txn_in_ledger {
            panic!(
                "The minimum number of low-fee transactions allowed per ledger \
                 (minimum_txn_in_ledger={}) exceeds the maximum (maximum_txn_in_ledger={})",
                setup.minimum_txn_in_ledger, max
            );
        }
    }

    tracing::info!(target: "bootstrap",
        ledgers_in_queue = setup.ledgers_in_queue,
        queue_size_min = setup.queue_size_min,
        minimum_txn_in_ledger = setup.minimum_txn_in_ledger,
        target_txn_in_ledger = setup.target_txn_in_ledger,
        maximum_txn_per_account = setup.maximum_txn_per_account,
        "Loaded [transaction_queue] config"
    );

    setup
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
