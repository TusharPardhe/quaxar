use xrpl_core::*;

#[test]
fn config_section_literals_match_current_cpp_config_sections_surface() {
    assert_eq!(ConfigSection::node_database(), "node_db");
    assert_eq!(ConfigSection::import_node_database(), "import_db");

    let expected = [
        (SECTION_AMENDMENTS, "amendments"),
        (SECTION_AMENDMENT_MAJORITY_TIME, "amendment_majority_time"),
        (SECTION_BETA_RPC_API, "beta_rpc_api"),
        (SECTION_CLUSTER_NODES, "cluster_nodes"),
        (SECTION_COMPRESSION, "compression"),
        (SECTION_DEBUG_LOGFILE, "debug_logfile"),
        (SECTION_ELB_SUPPORT, "elb_support"),
        (SECTION_FEE_DEFAULT, "fee_default"),
        (SECTION_FETCH_DEPTH, "fetch_depth"),
        (SECTION_INSIGHT, "insight"),
        (SECTION_IO_WORKERS, "io_workers"),
        (SECTION_IPS, "ips"),
        (SECTION_IPS_FIXED, "ips_fixed"),
        (SECTION_LEDGER_HISTORY, "ledger_history"),
        (SECTION_LEDGER_REPLAY, "ledger_replay"),
        (SECTION_MAX_TRANSACTIONS, "max_transactions"),
        (SECTION_NETWORK_ID, "network_id"),
        (SECTION_NETWORK_QUORUM, "network_quorum"),
        (SECTION_NODE_SEED, "node_seed"),
        (SECTION_NODE_SIZE, "node_size"),
        (SECTION_OVERLAY, "overlay"),
        (SECTION_PATH_SEARCH_OLD, "path_search_old"),
        (SECTION_PATH_SEARCH, "path_search"),
        (SECTION_PATH_SEARCH_FAST, "path_search_fast"),
        (SECTION_PATH_SEARCH_MAX, "path_search_max"),
        (SECTION_PEER_PRIVATE, "peer_private"),
        (SECTION_PEERS_MAX, "peers_max"),
        (SECTION_PEERS_IN_MAX, "peers_in_max"),
        (SECTION_PEERS_OUT_MAX, "peers_out_max"),
        (SECTION_PORT_GRPC, "port_grpc"),
        (SECTION_PREFETCH_WORKERS, "prefetch_workers"),
        (SECTION_REDUCE_RELAY, "reduce_relay"),
        (SECTION_RELATIONAL_DB, "relational_db"),
        (SECTION_RELAY_PROPOSALS, "relay_proposals"),
        (SECTION_RELAY_VALIDATIONS, "relay_validations"),
        (SECTION_RPC_STARTUP, "rpc_startup"),
        (SECTION_SIGNING_SUPPORT, "signing_support"),
        (SECTION_SNTP, "sntp_servers"),
        (SECTION_SSL_VERIFY, "ssl_verify"),
        (SECTION_SSL_VERIFY_FILE, "ssl_verify_file"),
        (SECTION_SSL_VERIFY_DIR, "ssl_verify_dir"),
        (SECTION_SERVER_DOMAIN, "server_domain"),
        (SECTION_SWEEP_INTERVAL, "sweep_interval"),
        (SECTION_VALIDATORS_FILE, "validators_file"),
        (SECTION_VALIDATION_SEED, "validation_seed"),
        (SECTION_VALIDATOR_KEYS, "validator_keys"),
        (SECTION_VALIDATOR_KEY_REVOCATION, "validator_key_revocation"),
        (SECTION_VALIDATOR_LIST_KEYS, "validator_list_keys"),
        (SECTION_VALIDATOR_LIST_SITES, "validator_list_sites"),
        (SECTION_VALIDATOR_LIST_THRESHOLD, "validator_list_threshold"),
        (SECTION_VALIDATORS, "validators"),
        (SECTION_VALIDATOR_TOKEN, "validator_token"),
        (SECTION_VETO_AMENDMENTS, "veto_amendments"),
        (SECTION_WORKERS, "workers"),
    ];

    assert_eq!(expected.len(), 54);
    for (actual, expected_literal) in expected {
        assert_eq!(actual, expected_literal);
    }
}
