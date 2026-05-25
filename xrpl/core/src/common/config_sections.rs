#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConfigSection;

impl ConfigSection {
    pub fn node_database() -> &'static str {
        "node_db"
    }

    pub fn import_node_database() -> &'static str {
        "import_db"
    }
}

pub const SECTION_AMENDMENTS: &str = "amendments";
pub const SECTION_AMENDMENT_MAJORITY_TIME: &str = "amendment_majority_time";
pub const SECTION_BETA_RPC_API: &str = "beta_rpc_api";
pub const SECTION_CLUSTER_NODES: &str = "cluster_nodes";
pub const SECTION_COMPRESSION: &str = "compression";
pub const SECTION_DEBUG_LOGFILE: &str = "debug_logfile";
pub const SECTION_ELB_SUPPORT: &str = "elb_support";
pub const SECTION_FEE_DEFAULT: &str = "fee_default";
pub const SECTION_FETCH_DEPTH: &str = "fetch_depth";
pub const SECTION_INSIGHT: &str = "insight";
pub const SECTION_IO_WORKERS: &str = "io_workers";
pub const SECTION_IPS: &str = "ips";
pub const SECTION_IPS_FIXED: &str = "ips_fixed";
pub const SECTION_LEDGER_HISTORY: &str = "ledger_history";
pub const SECTION_LEDGER_REPLAY: &str = "ledger_replay";
pub const SECTION_MAX_TRANSACTIONS: &str = "max_transactions";
pub const SECTION_NETWORK_ID: &str = "network_id";
pub const SECTION_NETWORK_QUORUM: &str = "network_quorum";
pub const SECTION_NODE_SEED: &str = "node_seed";
pub const SECTION_NODE_SIZE: &str = "node_size";
pub const SECTION_OVERLAY: &str = "overlay";
pub const SECTION_PATH_SEARCH_OLD: &str = "path_search_old";
pub const SECTION_PATH_SEARCH: &str = "path_search";
pub const SECTION_PATH_SEARCH_FAST: &str = "path_search_fast";
pub const SECTION_PATH_SEARCH_MAX: &str = "path_search_max";
pub const SECTION_PEER_PRIVATE: &str = "peer_private";
pub const SECTION_PEERS_MAX: &str = "peers_max";
pub const SECTION_PEERS_IN_MAX: &str = "peers_in_max";
pub const SECTION_PEERS_OUT_MAX: &str = "peers_out_max";
pub const SECTION_PORT_GRPC: &str = "port_grpc";
pub const SECTION_PREFETCH_WORKERS: &str = "prefetch_workers";
pub const SECTION_REDUCE_RELAY: &str = "reduce_relay";
pub const SECTION_RELATIONAL_DB: &str = "relational_db";
pub const SECTION_RELAY_PROPOSALS: &str = "relay_proposals";
pub const SECTION_RELAY_VALIDATIONS: &str = "relay_validations";
pub const SECTION_RPC_STARTUP: &str = "rpc_startup";
pub const SECTION_SIGNING_SUPPORT: &str = "signing_support";
pub const SECTION_SNTP: &str = "sntp_servers";
pub const SECTION_SSL_VERIFY: &str = "ssl_verify";
pub const SECTION_SSL_VERIFY_FILE: &str = "ssl_verify_file";
pub const SECTION_SSL_VERIFY_DIR: &str = "ssl_verify_dir";
pub const SECTION_SERVER_DOMAIN: &str = "server_domain";
pub const SECTION_SWEEP_INTERVAL: &str = "sweep_interval";
pub const SECTION_VALIDATORS_FILE: &str = "validators_file";
pub const SECTION_VALIDATION_SEED: &str = "validation_seed";
pub const SECTION_VALIDATOR_KEYS: &str = "validator_keys";
pub const SECTION_VALIDATOR_KEY_REVOCATION: &str = "validator_key_revocation";
pub const SECTION_VALIDATOR_LIST_KEYS: &str = "validator_list_keys";
pub const SECTION_VALIDATOR_LIST_SITES: &str = "validator_list_sites";
pub const SECTION_VALIDATOR_LIST_THRESHOLD: &str = "validator_list_threshold";
pub const SECTION_VALIDATORS: &str = "validators";
pub const SECTION_VALIDATOR_TOKEN: &str = "validator_token";
pub const SECTION_VETO_AMENDMENTS: &str = "veto_amendments";
pub const SECTION_WORKERS: &str = "workers";
