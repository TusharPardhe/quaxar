use super::{
    AcqMsg, AcqRegistry, AcqResult, CatchupResourceProfile, CompletedLedgerAcceptance,
    InboundEntry, InboundLedgers, InboundState, LedgerPublishAdvance, NodeStoreWriteMsg,
    PEERFINDER_MAX_CONNECT_ATTEMPTS, PEERFINDER_MAX_HOPS, PEERFINDER_NUMBER_OF_ENDPOINTS,
    PEERFINDER_RECENT_ATTEMPT_DURATION, PendingNodeStoreObject, RunDataLimiter,
    bind_server_runtime_into_root, bootstrap_acquire_budget_available, build_endpoint_broadcast,
    candidate_ledger_for_seq, candidate_reference_hash_from_reference_ledger,
    classify_completed_ledger_acceptance, classify_publish_advance,
    cold_bootstrap_persisted_validated_target, command_suggestions, current_ledger_is_fresh,
    first_command_like_arg, flush_nodestore_writes, hash_for_seq_from_reference_ledger,
    ledger_fetch_limit_override, node_store_usage_path, path_size_bytes, peerfinder_canonical_ip,
    peerfinder_outbound_target, preferred_closed_ledger_hash,
    preferred_closed_ledger_hash_from_hashes, promote_current_ledger, prune_known_endpoints,
    prune_recent_connect_attempts, remember_known_endpoint, select_autoconnect_endpoints,
    select_bootcache_endpoints, select_consensus_acquisition_target,
    select_post_acquisition_operating_mode, select_target_seq,
    should_attempt_completed_ledger_promotion, should_process_acquisition_tick,
    should_retry_publish_after_completed_history,
};
use app::{AppBootstrapOptions, build_bootstrap_root, load_basic_config_file};
use basics::base_uint::Uint256;
use ledger::Ledger;
use overlay::{Peer, PeerImp};
use protocol::{JsonValue, KeyType, SecretKey, derive_public_key};
use server::{
    RpcDispatcher, RpcReply, RpcRequest, RpcServerPortDeferredProtocol, ServerRuntime,
    ServerRuntimeBuildReport,
};
use std::collections::HashMap;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use xrpl_core::StartUpType;

#[derive(Clone)]
struct TestDispatcher;

impl RpcDispatcher for TestDispatcher {
    fn dispatch(&self, _request: RpcRequest<'_>) -> RpcReply {
        RpcReply::result(JsonValue::Object(Default::default()))
    }
}

fn config(text: &str) -> basics::basic_config::BasicConfig {
    let mut config = basics::basic_config::BasicConfig::new();
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
fn cli_unknown_command_helpers_detect_command_and_suggest_close_matches() {
    let args = vec![
        "xrpld".to_owned(),
        "--conf".to_owned(),
        "xrpld.cfg".to_owned(),
        "logs".to_owned(),
    ];
    assert_eq!(
        first_command_like_arg(&args, &["--conf", "-c", "--rpc-url"]),
        Some("logs")
    );

    let suggestions = command_suggestions(
        "logs",
        &["status", "log-level", "log-rotate", "ledger", "server-info"],
    );
    assert_eq!(suggestions, vec!["log-level", "log-rotate"]);
}

#[test]
fn xrpld_main_binds_server_runtime_into_the_composed_app_graph() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _guard = runtime.enter();
    let dir = TempDir::new().expect("tempdir");
    let config_path = dir.path().join("xrpld.cfg");
    let database_path = dir.path().join("sql");
    let node_db_path = dir.path().join("node-db");
    fs::write(
        &config_path,
        format!(
            r#"
[ledger_history]
128

[workers]
4

[io_workers]
2

[database_path]
{}

[server]
port_rpc
port_peer

[port_rpc]
ip = 127.0.0.1
port = 5005
protocol = http

[port_peer]
ip = 0.0.0.0
port = 51235
protocol = peer
limit = 64

[node_db]
type = Memory
path = {}
"#,
            database_path.display(),
            node_db_path.display(),
        ),
    )
    .expect("config file");

    let config = load_basic_config_file(&config_path).expect("config should parse");
    let bootstrap = build_bootstrap_root(
        &config,
        &AppBootstrapOptions {
            config_path: config_path.clone(),
            standalone: false,
            start_valid: true,
            elb_support: true,
            io_threads: 1,
            job_queue_threads: 1,
            start_type: StartUpType::Fresh,
            ..AppBootstrapOptions::default()
        },
    )
    .expect("bootstrap root should build");
    let mut report = bootstrap.report;
    let mut root = bootstrap.root;
    let runtime = ServerRuntime::new(
        root.basic_app().handle().clone(),
        TestDispatcher,
        Vec::new(),
    );
    bind_server_runtime_into_root(
        &mut root,
        &mut report,
        ServerRuntimeBuildReport {
            runtime,
            deferred_protocols: vec![RpcServerPortDeferredProtocol {
                port_name: "port_peer".to_owned(),
                protocol: "peer".to_owned(),
                reason: "test handoff".to_owned(),
            }],
        },
        None,
        None,
        None,
    );

    assert!(report.has_server_runtime);
    assert_eq!(
        report.server_configured_ports,
        vec!["port_rpc".to_owned(), "port_peer".to_owned()]
    );
    assert_eq!(
        report.deferred_protocols,
        vec!["peer on port_peer".to_owned()]
    );
    assert!(root.runtime_bindings().server.is_some());
    assert_eq!(
        root.server_handler().snapshot().configured_ports,
        vec!["port_rpc".to_owned(), "port_peer".to_owned()]
    );
    assert_eq!(
        root.server_handler().snapshot().deferred_protocols,
        vec!["peer on port_peer".to_owned()]
    );
    assert!(!root.server_handler().snapshot().started);
}

#[test]
fn node_store_usage_path_uses_the_configured_node_db_directory() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _guard = runtime.enter();
    let dir = TempDir::new().expect("tempdir");
    let config_path = dir.path().join("xrpld.cfg");
    let database_path = dir.path().join("sql");
    let node_db_path = dir.path().join("node-db");
    fs::write(
        &config_path,
        format!(
            r#"
[database_path]
{}

[node_db]
type = Memory
path = {}
"#,
            database_path.display(),
            node_db_path.display(),
        ),
    )
    .expect("config file");

    let config = load_basic_config_file(&config_path).expect("config should parse");
    let _bootstrap = build_bootstrap_root(
        &config,
        &AppBootstrapOptions {
            config_path,
            start_valid: true,
            ..AppBootstrapOptions::default()
        },
    )
    .expect("bootstrap root should build");

    assert_eq!(node_store_usage_path(&config), Some(node_db_path));
}

#[test]
fn promote_current_ledger_keeps_runtime_and_app_published_ledgers_aligned() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _guard = runtime.enter();
    let dir = TempDir::new().expect("tempdir");
    let config_path = dir.path().join("xrpld.cfg");
    let database_path = dir.path().join("sql");
    let node_db_path = dir.path().join("node-db");
    fs::write(
        &config_path,
        format!(
            r#"
[database_path]
{}

[node_db]
type = Memory
path = {}
"#,
            database_path.display(),
            node_db_path.display(),
        ),
    )
    .expect("config file");

    let config = load_basic_config_file(&config_path).expect("config should parse");
    let bootstrap = build_bootstrap_root(
        &config,
        &AppBootstrapOptions {
            config_path,
            standalone: false,
            start_valid: true,
            io_threads: 1,
            job_queue_threads: 1,
            start_type: StartUpType::Fresh,
            ..AppBootstrapOptions::default()
        },
    )
    .expect("bootstrap root should build");
    let root = bootstrap.root;
    let mut ledger = ledger::Ledger::from_ledger_seq_and_close_time(1_234, 5_678, false);
    ledger.set_immutable(true);
    let ledger = std::sync::Arc::new(ledger);

    promote_current_ledger(&root, &[], std::sync::Arc::clone(&ledger));

    assert_eq!(root.published_ledger_seq(), Some(1_234));
    assert_eq!(root.validated_ledger_seq(), Some(1_234));
    assert!(!root.need_network_ledger());
    assert_eq!(
        root.ledger_master_runtime()
            .expect("ledger master runtime")
            .ledger_master()
            .published_ledger()
            .as_ref()
            .map(|ledger| ledger.header().seq),
        Some(1_234)
    );
}

#[test]
fn path_size_bytes_counts_rotating_backend_directories() {
    let dir = TempDir::new().expect("tempdir");
    let writable = dir.path().join("xrpldb.0000");
    let archive = dir.path().join("xrpldb.0001");
    fs::create_dir(&writable).expect("writable dir");
    fs::create_dir(&archive).expect("archive dir");
    fs::write(writable.join("nudb.dat"), vec![0_u8; 128]).expect("writable data");
    fs::write(archive.join("nudb.dat"), vec![0_u8; 256]).expect("archive data");

    assert_eq!(path_size_bytes(dir.path()), 384);
}

#[test]
fn select_target_seq_prefers_latest_shared_seq_during_bootstrap() {
    assert_eq!(select_target_seq(1, true, 160, None), 160);
}

#[test]
fn select_target_seq_keeps_post_bootstrap_progress_bounded_by_floor_and_ceiling() {
    assert_eq!(select_target_seq(150, true, 170, None), 151);
    assert_eq!(select_target_seq(150, false, 170, None), 151);
    assert_eq!(select_target_seq(170, true, 170, None), 0);
}

#[test]
fn select_target_seq_stays_on_next_sequential_ledger_even_when_peer_floor_is_higher() {
    assert_eq!(select_target_seq(17089668, true, 17090268, None), 17089669);
}

#[test]
fn cold_bootstrap_does_not_pin_persisted_validated_anchor() {
    let hash = Uint256::from_array([0x42; 32]);

    assert_eq!(
        cold_bootstrap_persisted_validated_target(0, Some((hash, 17264245))),
        None
    );
    assert_eq!(
        cold_bootstrap_persisted_validated_target(17264245, Some((hash, 17264245))),
        None
    );
    assert_eq!(
        cold_bootstrap_persisted_validated_target(0, Some((Uint256::default(), 17264245))),
        None
    );
}

#[test]
fn select_consensus_acquisition_target_prefers_latest_trusted_ledger_during_bootstrap() {
    let earliest = Uint256::from_array([1; 32]);
    let latest = Uint256::from_array([2; 32]);
    let targets = vec![(latest, 220), (earliest, 180)];

    assert_eq!(
        select_consensus_acquisition_target(1, &targets),
        Some((latest, 220))
    );
}

#[test]
fn select_consensus_acquisition_target_prefers_lowest_seq_when_already_near_tip() {
    let lower = Uint256::from_array([3; 32]);
    let higher = Uint256::from_array([4; 32]);
    let targets = vec![(higher, 204), (lower, 201)];

    assert_eq!(
        select_consensus_acquisition_target(200, &targets),
        Some((lower, 201))
    );
}

#[test]
fn select_consensus_acquisition_target_prefers_latest_seq_for_large_catchup_gaps() {
    let older = Uint256::from_array([5; 32]);
    let newer = Uint256::from_array([6; 32]);
    let targets = vec![(older, 260), (newer, 320)];

    assert_eq!(
        select_consensus_acquisition_target(200, &targets),
        Some((newer, 320))
    );
}

#[test]
fn poll_results_keeps_completed_entry_when_sender_disconnects_after_completion() {
    let registry: AcqRegistry = Arc::new(std::sync::Mutex::new(HashMap::new()));
    let tree_cache = Arc::new(shamap::tree_node_cache::TreeNodeCache::new(
        "test-acq",
        8,
        time::Duration::seconds(1),
        basics::tagged_cache::MonotonicClock::default(),
    ));
    let full_below = Arc::new(shamap::family::FullBelowCacheImpl::new(
        1,
        basics::tagged_cache::MonotonicClock::default(),
        basics::hardened_hash::HardenedHashBuilder::default(),
        8,
    ));
    let fetch_pack = Arc::new(ledger::FetchPackCache::new(
        8,
        time::Duration::seconds(1),
        basics::tagged_cache::MonotonicClock::default(),
    ));
    let run_data_limiter = Arc::new(RunDataLimiter::new(1));
    let shared_stored = Arc::new(basics::tagged_cache::KeyCache::new(
        "test-acq-write-dedup",
        8,
        time::Duration::seconds(1),
        basics::tagged_cache::MonotonicClock::default(),
    ));
    let mut inbound_ledgers = InboundLedgers::new(
        registry,
        tree_cache,
        full_below,
        fetch_pack,
        run_data_limiter,
        shared_stored,
    );

    let ledger = Ledger::from_ledger_seq_and_close_time(99, 777, false);
    let hash = *ledger.header().hash.as_uint256();
    let (tx, _rx) = std::sync::mpsc::channel::<AcqMsg>();
    let (result_tx, result_rx) = std::sync::mpsc::channel::<AcqResult>();
    result_tx
        .send(AcqResult::Complete(ledger.clone()))
        .expect("result channel should accept completion");
    drop(result_tx);

    inbound_ledgers.entries.insert(
        hash,
        InboundEntry {
            seq: ledger.header().seq,
            tx,
            result_rx,
            handle: std::thread::spawn(|| {}),
            last_touched: Instant::now(),
            state: InboundState::InProgress,
            skip_state: false,
        },
    );

    let completed = inbound_ledgers.poll_results();

    assert_eq!(completed.len(), 1);
    assert!(inbound_ledgers.recent_failures.is_empty());
    let entry = inbound_ledgers
        .entries
        .get(&hash)
        .expect("entry should remain");
    assert!(matches!(entry.state, InboundState::Complete(_)));
}

#[test]
fn hash_for_seq_from_reference_ledger_walks_back_from_completed_higher_ledger() {
    let config = ledger::LedgerConfig::default();
    let genesis = Ledger::create_genesis(false, &config, []).expect("genesis ledger");
    let mut history = vec![genesis];

    for close_time in 1..=20u32 {
        let mut next = Ledger::from_previous(
            history
                .last()
                .expect("history must contain previous ledger"),
            close_time,
        );
        next.update_skip_list().expect("skip list should update");
        history.push(next);
    }

    let latest = history.last().expect("latest ledger");
    let target = history[10].header().seq;
    let expected_hash = history[10].header().hash;

    assert_eq!(
        hash_for_seq_from_reference_ledger(latest, target),
        Some(expected_hash)
    );
}

#[test]
fn hash_for_seq_from_reference_ledger_returns_none_for_future_targets() {
    let ledger = Ledger::from_ledger_seq_and_close_time(88, 777, false);

    assert_eq!(hash_for_seq_from_reference_ledger(&ledger, 89), None);
}

#[test]
fn candidate_ledger_for_seq_boundary_rule() {
    assert_eq!(candidate_ledger_for_seq(104053598), 104053760);
    assert_eq!(candidate_ledger_for_seq(256), 256);
    assert_eq!(candidate_ledger_for_seq(257), 512);
}

#[test]
fn candidate_reference_hash_from_reference_ledger_uses_256_boundary_when_direct_lookup_fails() {
    let config = ledger::LedgerConfig::default();
    let genesis = Ledger::create_genesis(false, &config, []).expect("genesis ledger");
    let mut history = vec![genesis];

    for close_time in 1..=420u32 {
        let mut next = Ledger::from_previous(
            history
                .last()
                .expect("history must contain previous ledger"),
            close_time,
        );
        next.update_skip_list().expect("skip list should update");
        history.push(next);
    }

    let latest = history.last().expect("latest ledger");
    let target_seq = 10u32;
    let candidate_seq = candidate_ledger_for_seq(target_seq);
    let expected_hash = latest
        .hash_of_seq(candidate_seq, &ledger::NullLedgerJournal)
        .expect("candidate sequence hash should be available");

    assert_eq!(
        hash_for_seq_from_reference_ledger(latest, target_seq),
        None,
        "direct lookup should fail once the target is more than 256 ledgers behind and not on a 256 boundary"
    );
    assert_eq!(
        candidate_reference_hash_from_reference_ledger(latest, target_seq),
        Some((candidate_seq, expected_hash))
    );
}

#[test]
fn preferred_closed_ledger_hash_prefers_majority_peer_hash() {
    let ours = Uint256::from_array([1; 32]);
    let peer = Uint256::from_array([2; 32]);

    assert_eq!(
        preferred_closed_ledger_hash_from_hashes([peer, peer, ours], ours, true),
        Some(peer)
    );
}

#[test]
fn completed_ledger_promotion_check_accept_sequence_guard() {
    assert!(!should_attempt_completed_ledger_promotion(
        104055711, 104055713
    ));
    assert!(!should_attempt_completed_ledger_promotion(
        104055713, 104055713
    ));
    assert!(should_attempt_completed_ledger_promotion(
        104055714, 104055713
    ));
}

#[test]
fn completed_acquisition_acceptance_classifies_live_backfill_and_far_ahead_cases() {
    assert_eq!(
        classify_completed_ledger_acceptance(104056343, 104056344, false, false),
        CompletedLedgerAcceptance::HistoricalCached,
        "completed historical/backfill ledgers are cache-only like reference checkAccept"
    );
    assert_eq!(
        classify_completed_ledger_acceptance(104056727, 104056344, false, false),
        CompletedLedgerAcceptance::HeldForQuorum,
        "far-ahead post-bootstrap ledgers must not become validated without quorum"
    );
    assert_eq!(
        classify_completed_ledger_acceptance(104056727, 104056344, false, true),
        CompletedLedgerAcceptance::ValidatedAccepted,
        "a future candidate can promote only after the reference checkAccept gate passes"
    );
    assert_eq!(
        classify_completed_ledger_acceptance(104056727, 0, false, false),
        CompletedLedgerAcceptance::HeldForQuorum,
        "bootstrap is no longer a validationless peer-accept exception"
    );
    assert_eq!(
        classify_completed_ledger_acceptance(104056727, 0, true, false),
        CompletedLedgerAcceptance::HeldForQuorum,
        "skip-state ledgers also require the reference checkAccept gate"
    );
}

#[test]
fn publish_advance_classification_gap_rules() {
    assert_eq!(
        classify_publish_advance(104056727, None),
        LedgerPublishAdvance::FirstPublished,
        "reference publishes the first validated ledger directly"
    );
    assert_eq!(
        classify_publish_advance(104056727, Some(104056626)),
        LedgerPublishAdvance::GapTooLarge,
        "reference jumps directly when the validated-to-published gap exceeds 100"
    );
    assert_eq!(
        classify_publish_advance(104056727, Some(104056627)),
        LedgerPublishAdvance::Sequential,
        "a 100-ledger gap is still filled sequentially"
    );
    assert_eq!(
        classify_publish_advance(104056727, Some(104056727)),
        LedgerPublishAdvance::NothingToPublish
    );
}

#[test]
fn historical_fill_retries_publish_when_it_closes_validated_gap() {
    assert!(
        should_retry_publish_after_completed_history(104059398, Some(104059397), 104059455),
        "reference doAdvance retries when the next unpublished ledger becomes available"
    );
    assert!(
        !should_retry_publish_after_completed_history(104059397, Some(104059397), 104059455),
        "already-published history must not trigger another publish pass"
    );
    assert!(
        !should_retry_publish_after_completed_history(104059456, Some(104059397), 104059455),
        "history beyond the validated ledger cannot be published yet"
    );
}

#[test]
fn current_ledger_is_fresh_two_resolution_window() {
    assert!(current_ledger_is_fresh(159, 100, 30));
    assert!(!current_ledger_is_fresh(160, 100, 30));
}

#[test]
fn select_post_acquisition_operating_mode_promotes_to_tracking_before_full() {
    assert_eq!(
        select_post_acquisition_operating_mode(
            app::NetworkOpsOperatingMode::Syncing,
            false,
            false,
            false,
        ),
        app::NetworkOpsOperatingMode::Tracking
    );
    assert_eq!(
        select_post_acquisition_operating_mode(
            app::NetworkOpsOperatingMode::Connected,
            false,
            false,
            true,
        ),
        app::NetworkOpsOperatingMode::Full
    );
}

#[test]
fn select_post_acquisition_operating_mode_blocks_full_on_ledger_change_or_staleness() {
    assert_eq!(
        select_post_acquisition_operating_mode(
            app::NetworkOpsOperatingMode::Connected,
            false,
            true,
            true,
        ),
        app::NetworkOpsOperatingMode::Connected
    );
    assert_eq!(
        select_post_acquisition_operating_mode(
            app::NetworkOpsOperatingMode::Connected,
            false,
            false,
            false,
        ),
        app::NetworkOpsOperatingMode::Tracking
    );
    assert_eq!(
        select_post_acquisition_operating_mode(
            app::NetworkOpsOperatingMode::Connected,
            true,
            false,
            true,
        ),
        app::NetworkOpsOperatingMode::Connected
    );
}

#[test]
fn select_post_acquisition_operating_mode_blocks_tracking_when_peers_disagree() {
    assert_eq!(
        select_post_acquisition_operating_mode(
            app::NetworkOpsOperatingMode::Syncing,
            false,
            true,
            true,
        ),
        app::NetworkOpsOperatingMode::Syncing
    );
}

#[test]
fn preferred_closed_ledger_hash_prefers_trusted_validations_over_peer_majority() {
    let our_closed = Uint256::from_u64(11);
    let prev_closed = Uint256::from_u64(10);
    let peer_majority = Uint256::from_u64(99);
    let trusted = Uint256::from_u64(42);

    assert_eq!(
        preferred_closed_ledger_hash(
            Some((150, trusted)),
            120,
            [peer_majority, peer_majority, our_closed],
            our_closed,
            prev_closed,
            true,
        ),
        trusted
    );
}

#[test]
fn preferred_closed_ledger_hash_keeps_our_lcl_when_trusted_preference_is_too_old() {
    let our_closed = Uint256::from_u64(11);
    let prev_closed = Uint256::from_u64(10);
    let stale_trusted = Uint256::from_u64(42);

    assert_eq!(
        preferred_closed_ledger_hash(
            Some((80, stale_trusted)),
            120,
            [our_closed],
            our_closed,
            prev_closed,
            true,
        ),
        our_closed
    );
}

#[test]
fn preferred_closed_ledger_hash_wont_switch_to_our_previous_lcl() {
    let our_closed = Uint256::from_u64(11);
    let prev_closed = Uint256::from_u64(10);

    assert_eq!(
        preferred_closed_ledger_hash(
            None,
            0,
            [prev_closed, prev_closed, prev_closed],
            our_closed,
            prev_closed,
            true,
        ),
        our_closed
    );
}

#[test]
fn remember_known_endpoint_keeps_lowest_hops_and_refreshes_timestamp() {
    let mut known = std::collections::HashMap::new();
    let endpoint = "127.0.0.1:51235".parse().expect("endpoint");
    let first_seen = Instant::now();
    let refreshed = first_seen + Duration::from_secs(5);

    remember_known_endpoint(&mut known, endpoint, 3, first_seen);
    remember_known_endpoint(&mut known, endpoint, 5, refreshed);

    let entry = known.get(&endpoint).expect("known endpoint");
    assert_eq!(entry.hops, 3);
    assert_eq!(entry.last_seen, refreshed);
}

#[test]
fn prune_known_endpoints_expires_stale_entries_livecache() {
    let fresh_endpoint = "127.0.0.1:51235".parse().expect("fresh endpoint");
    let stale_endpoint = "127.0.0.2:51235".parse().expect("stale endpoint");
    let now = Instant::now();
    let mut known = std::collections::HashMap::new();

    remember_known_endpoint(&mut known, fresh_endpoint, 1, now - Duration::from_secs(10));
    remember_known_endpoint(&mut known, stale_endpoint, 2, now - Duration::from_secs(31));

    prune_known_endpoints(&mut known, now);

    assert!(known.contains_key(&fresh_endpoint));
    assert!(!known.contains_key(&stale_endpoint));
}

#[test]
fn build_endpoint_broadcast_limits_entries_and_includes_self_once() {
    let now = Instant::now();
    let mut known = std::collections::HashMap::new();
    for index in 0..20u16 {
        let endpoint = format!("127.0.0.{}:{}", index + 1, 52000 + index)
            .parse()
            .expect("endpoint");
        remember_known_endpoint(
            &mut known,
            endpoint,
            1 + (index as u32 % PEERFINDER_MAX_HOPS),
            now + Duration::from_secs(index as u64),
        );
    }

    let secret = SecretKey::from_bytes([21u8; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    let peer = PeerImp::new(
        21,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51235),
        public,
        "peer-21",
    );
    let peer_dyn: Arc<dyn Peer> = peer.clone();

    let broadcast = build_endpoint_broadcast(Some(51235), &known, &peer_dyn, now);
    assert_eq!(broadcast.len(), PEERFINDER_NUMBER_OF_ENDPOINTS);
    assert_eq!(broadcast[0].hops, 0);
    assert_eq!(broadcast[0].endpoint, "[::]:51235");
    assert!(broadcast.iter().skip(1).all(|endpoint| endpoint.hops > 0));
}

#[test]
fn build_endpoint_broadcast_filters_recent_entries_and_remote_ip_per_peer() {
    let now = Instant::now();
    let mut known = std::collections::HashMap::new();
    let secret = SecretKey::from_bytes([22u8; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    let peer = PeerImp::new(
        22,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)), 51235),
        public,
        "peer-22",
    );
    let peer_dyn: Arc<dyn Peer> = peer.clone();

    let remote_ip_endpoint = "10.0.0.2:51236".parse().expect("peer endpoint");
    let repeated_endpoint = "10.0.0.3:51235".parse().expect("recent endpoint");
    let duplicate_ip_endpoint = "10.0.0.3:51236".parse().expect("same ip endpoint");

    remember_known_endpoint(&mut known, remote_ip_endpoint, 1, now);
    remember_known_endpoint(
        &mut known,
        repeated_endpoint,
        1,
        now + Duration::from_secs(1),
    );
    remember_known_endpoint(
        &mut known,
        duplicate_ip_endpoint,
        1,
        now + Duration::from_secs(2),
    );
    peer.remember_recent_endpoint(repeated_endpoint, 1, now, Duration::from_secs(30));

    let broadcast = build_endpoint_broadcast(Some(51235), &known, &peer_dyn, now);
    assert!(
        !broadcast
            .iter()
            .any(|endpoint| endpoint.endpoint == remote_ip_endpoint.to_string())
    );
    assert!(
        !broadcast
            .iter()
            .any(|endpoint| endpoint.endpoint == repeated_endpoint.to_string())
    );
    assert_eq!(
        broadcast
            .iter()
            .filter(|endpoint| endpoint.endpoint.starts_with("10.0.0.3:"))
            .count(),
        1
    );
}

#[test]
fn select_autoconnect_endpoints_prefers_low_hops_unique_ips_and_caps_attempts() {
    let now = Instant::now();
    let mut known = std::collections::HashMap::new();
    let already_connected: std::collections::HashSet<std::net::IpAddr> =
        ["127.0.0.1".parse().expect("connected ip")]
            .into_iter()
            .collect();

    remember_known_endpoint(
        &mut known,
        "127.0.0.1:51235".parse().expect("existing peer"),
        1,
        now,
    );
    remember_known_endpoint(
        &mut known,
        "127.0.0.2:51235".parse().expect("candidate 1"),
        2,
        now + Duration::from_secs(1),
    );
    remember_known_endpoint(
        &mut known,
        "127.0.0.2:51236".parse().expect("same ip duplicate"),
        1,
        now + Duration::from_secs(2),
    );
    remember_known_endpoint(
        &mut known,
        "127.0.0.3:51235".parse().expect("higher-hop candidate"),
        3,
        now + Duration::from_secs(3),
    );
    remember_known_endpoint(
        &mut known,
        "127.0.0.4:51235".parse().expect("too many hops"),
        PEERFINDER_MAX_HOPS + 1,
        now + Duration::from_secs(4),
    );

    for index in 5..13u16 {
        let endpoint = format!("127.0.1.{}:{}", index, 52000 + index)
            .parse()
            .expect("bulk candidate");
        remember_known_endpoint(
            &mut known,
            endpoint,
            1,
            now + Duration::from_secs(index as u64),
        );
    }

    let selected = select_autoconnect_endpoints(&already_connected, &known, &HashMap::new(), now);
    assert!(
        !selected
            .iter()
            .any(|endpoint| endpoint.ip().to_string() == "127.0.0.1")
    );
    assert!(
        selected
            .iter()
            .any(|endpoint| endpoint.ip().to_string() == "127.0.0.3")
    );
    assert!(
        !selected
            .iter()
            .any(|endpoint| endpoint.ip().to_string() == "127.0.0.4")
    );
    assert_eq!(
        selected
            .iter()
            .filter(|endpoint| endpoint.ip().to_string() == "127.0.0.2")
            .count(),
        1
    );
    assert!(selected.len() <= PEERFINDER_MAX_CONNECT_ATTEMPTS);
}

#[test]
fn select_autoconnect_endpoints_skips_recent_attempted_ips_squelch() {
    let now = Instant::now();
    let mut known = std::collections::HashMap::new();
    let first = "10.0.0.10:51235".parse().expect("first");
    let second = "10.0.0.11:51235".parse().expect("second");
    remember_known_endpoint(&mut known, first, 1, now);
    remember_known_endpoint(&mut known, second, 1, now);
    let recent_attempts = HashMap::from([(first.ip(), now + PEERFINDER_RECENT_ATTEMPT_DURATION)]);

    let selected = select_autoconnect_endpoints(
        &std::collections::HashSet::new(),
        &known,
        &recent_attempts,
        now,
    );

    assert_eq!(selected, vec![second]);
}

#[test]
fn select_autoconnect_endpoints_normalizes_ipv4_mapped_addresses() {
    let now = Instant::now();
    let mut known = std::collections::HashMap::new();
    let mapped: std::net::SocketAddr = "[::ffff:10.0.0.10]:51235".parse().expect("mapped");
    let v4: std::net::SocketAddr = "10.0.0.10:51236".parse().expect("v4");
    remember_known_endpoint(&mut known, mapped, 1, now);
    remember_known_endpoint(&mut known, v4, 1, now + Duration::from_secs(1));

    let selected = select_autoconnect_endpoints(
        &std::collections::HashSet::from([peerfinder_canonical_ip(mapped.ip())]),
        &known,
        &HashMap::new(),
        now,
    );

    assert!(selected.is_empty());
}

#[test]
fn select_bootcache_endpoints_uses_redirects_without_rebroadcasting_live_hops() {
    let now = Instant::now();
    let connected = std::collections::HashSet::from(["10.0.0.1".parse().expect("connected")]);
    let first: std::net::SocketAddr = "10.0.0.1:51235".parse().expect("already connected");
    let second: std::net::SocketAddr = "10.0.0.2:51235".parse().expect("candidate");
    let duplicate_ip: std::net::SocketAddr = "10.0.0.2:51236".parse().expect("duplicate ip");
    let squelched: std::net::SocketAddr = "10.0.0.3:51235".parse().expect("squelched");
    let bootcache = std::collections::BTreeMap::from([
        (first, 0),
        (second, 2),
        (duplicate_ip, 5),
        (squelched, 3),
    ]);
    let recent_attempts =
        HashMap::from([(squelched.ip(), now + PEERFINDER_RECENT_ATTEMPT_DURATION)]);

    let selected = select_bootcache_endpoints(&connected, &bootcache, &recent_attempts, now);

    assert_eq!(selected, vec![duplicate_ip]);
}

#[test]
fn prune_recent_connect_attempts_expires_cpp_squelch_window() {
    let now = Instant::now();
    let stale_ip = "10.0.0.10".parse().expect("stale");
    let fresh_ip = "10.0.0.11".parse().expect("fresh");
    let mut attempts = HashMap::from([
        (stale_ip, now - Duration::from_secs(1)),
        (fresh_ip, now + Duration::from_secs(1)),
    ]);

    prune_recent_connect_attempts(&mut attempts, now);

    assert!(!attempts.contains_key(&stale_ip));
    assert!(attempts.contains_key(&fresh_ip));
}

#[test]
fn peerfinder_outbound_target_percent_and_minimum_shape() {
    assert_eq!(peerfinder_outbound_target(21, true), 10);
    assert_eq!(peerfinder_outbound_target(64, true), 10);
    assert_eq!(peerfinder_outbound_target(100, true), 15);
    assert_eq!(peerfinder_outbound_target(8, true), 8);
    assert_eq!(peerfinder_outbound_target(21, false), 21);
}

#[test]
fn catchup_resource_profile_tiny_keeps_local_worker_limits_conservative() {
    let profile = CatchupResourceProfile::for_node_size(Some("tiny"));
    assert_eq!(profile.run_data_concurrency, 2);
    assert_eq!(profile.acq_tree_cache_size, 262_144);
    assert_eq!(profile.acq_tree_cache_age_seconds, 30);
    assert_eq!(profile.ledger_fetch_limit, 2);
    assert!(profile.acq_fetch_pack_size < 10_000);
}

#[test]
fn catchup_resource_profile_defaults_to_medium_when_missing() {
    let profile = CatchupResourceProfile::for_node_size(None);
    assert_eq!(profile.run_data_concurrency, 6);
    assert_eq!(profile.acq_tree_cache_size, 2_097_152);
    assert_eq!(profile.acq_tree_cache_age_seconds, 90);
    assert_eq!(profile.ledger_fetch_limit, 4);
}

#[test]
fn catchup_resource_profile_applies_ledger_fetch_limit_override() {
    let tiny = CatchupResourceProfile::for_node_size(Some("tiny"))
        .with_ledger_fetch_limit_override(Some(8));
    assert_eq!(tiny.run_data_concurrency, 2);
    assert_eq!(tiny.acq_tree_cache_size, 262_144);
    assert_eq!(tiny.ledger_fetch_limit, 8);

    let medium =
        CatchupResourceProfile::for_node_size(None).with_ledger_fetch_limit_override(Some(1));
    assert_eq!(medium.run_data_concurrency, 6);
    assert_eq!(medium.ledger_fetch_limit, 1);
}

#[test]
fn bootstrap_acquire_budget_ledger_fetch_shape() {
    assert!(bootstrap_acquire_budget_available(0, 0, 5, false));
    assert!(!bootstrap_acquire_budget_available(0, 1, 5, false));
    assert!(bootstrap_acquire_budget_available(0, 5, 5, true));
    assert!(bootstrap_acquire_budget_available(2, 1, 2, false));
    assert!(!bootstrap_acquire_budget_available(2, 2, 2, false));
}

#[test]
fn ledger_fetch_limit_override_parses_optional_expert_config() {
    assert_eq!(
        ledger_fetch_limit_override(&config("")).expect("missing section"),
        None
    );
    assert_eq!(
        ledger_fetch_limit_override(&config("[ledger_acquisition]\nledger_fetch_limit = 8\n"))
            .expect("valid override"),
        Some(8)
    );
    assert_eq!(
        ledger_fetch_limit_override(&config("[ledger_acquisition]\n")).expect("missing key"),
        None
    );

    let zero =
        ledger_fetch_limit_override(&config("[ledger_acquisition]\nledger_fetch_limit = 0\n"))
            .expect_err("zero rejected");
    assert!(zero.contains("between 1 and 8"));

    let high =
        ledger_fetch_limit_override(&config("[ledger_acquisition]\nledger_fetch_limit = 9\n"))
            .expect_err("too high rejected");
    assert!(high.contains("between 1 and 8"));

    let invalid =
        ledger_fetch_limit_override(&config("[ledger_acquisition]\nledger_fetch_limit = many\n"))
            .expect_err("invalid rejected");
    assert_eq!(
        invalid,
        "Configured ledger_acquisition.ledger_fetch_limit is invalid"
    );
}

#[test]
fn acquisition_tick_processes_initial_peer_updates_before_timeout() {
    assert!(should_process_acquisition_tick(
        false, false, false, true, true
    ));
    assert!(!should_process_acquisition_tick(
        false, false, false, true, false
    ));
    assert!(!should_process_acquisition_tick(
        false, false, false, false, true
    ));
}

#[test]
fn flush_nodestore_writes_waits_for_prior_messages() {
    let (tx, rx) = std::sync::mpsc::channel::<NodeStoreWriteMsg>();
    let writes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let pending = std::sync::Arc::new(std::sync::Mutex::new(HashMap::<
        Uint256,
        PendingNodeStoreObject,
    >::new()));
    let writes_for_worker = std::sync::Arc::clone(&writes);
    let pending_for_worker = std::sync::Arc::clone(&pending);
    let worker = std::thread::spawn(move || {
        loop {
            match rx.recv().expect("worker message") {
                NodeStoreWriteMsg::Write {
                    obj_type,
                    data,
                    hash,
                    ..
                } => {
                    writes_for_worker.lock().expect("writes mutex").push(hash);
                    pending_for_worker
                        .lock()
                        .expect("pending mutex")
                        .remove(&hash)
                        .or_else(|| {
                            Some(PendingNodeStoreObject {
                                obj_type,
                                data,
                                hash,
                            })
                        });
                }
                NodeStoreWriteMsg::Flush(ack) => {
                    let _ = ack.send(());
                }
                NodeStoreWriteMsg::Stop => break,
            }
        }
    });

    for fill in [0x11u8, 0x22, 0x33] {
        let hash = Uint256::from_array([fill; 32]);
        pending.lock().expect("pending mutex").insert(
            hash,
            PendingNodeStoreObject {
                obj_type: nodestore::NodeObjectType::AccountNode,
                data: vec![fill],
                hash,
            },
        );
        tx.send(NodeStoreWriteMsg::Write {
            obj_type: nodestore::NodeObjectType::AccountNode,
            data: vec![fill],
            hash,
            seq: u32::from(fill),
        })
        .expect("write message");
    }

    assert!(flush_nodestore_writes(&tx));
    assert_eq!(writes.lock().expect("writes mutex").len(), 3);
    assert!(pending.lock().expect("pending mutex").is_empty());

    tx.send(NodeStoreWriteMsg::Stop).expect("stop message");
    worker.join().expect("worker join");
}
