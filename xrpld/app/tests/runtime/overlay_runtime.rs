use app::{
    AppBootstrapOptions, ApplicationRoot, BootstrapOverlayHandoff, ManagedComponent,
    ServiceRegistry, build_bootstrap_runtime, build_overlay_runtime, build_overlay_setup,
};
use basics::basic_config::BasicConfig;
use overlay::{Overlay, Peer, PeerImp};
use protocol::{KeyType, PublicKey, SecretKey, derive_public_key, parse_base58_node_public};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tempfile::TempDir;
use xrpl_core::{NetworkIDService, PeerReservation};

const CLUSTER_NODE_PUBLIC: &str = "n94a1u4jAz288pZLtw6yFWVbi89YamiC6JBXPVUj5zmExe5fTVg9";

fn config_from_text(text: &str) -> BasicConfig {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("xrpld.cfg");
    fs::write(&path, text).expect("config file");
    app::load_basic_config_file(path).expect("config")
}

fn write_config(path: &Path, text: &str) -> BasicConfig {
    fs::write(path, text).expect("config file");
    app::load_basic_config_file(path).expect("config")
}

fn overlay_public_key(seed: u8) -> PublicKey {
    let secret = SecretKey::from_bytes([seed; 32]);
    derive_public_key(KeyType::Secp256k1, &secret).expect("public key")
}

fn overlay_peer(id: u32, seed: u8) -> Arc<PeerImp> {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    PeerImp::new(
        id,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5000 + id as u16),
        overlay_public_key(seed),
        format!("peer-{id}"),
    )
}

fn cluster_public_key() -> PublicKey {
    let bytes = parse_base58_node_public(CLUSTER_NODE_PUBLIC).expect("cluster node public");
    PublicKey::from_slice(&bytes).expect("cluster key")
}

fn configured_overlay_text() -> &'static str {
    r#"
[server]
port_rpc
port_peer

[port_rpc]
ip = 127.0.0.1
port = 5005
protocol = http,ws

[port_peer]
ip = 0.0.0.0
port = 51235
protocol = peer
limit = 64

[overlay]
ip_limit = 32

[network_id]
21338

[cluster_nodes]
n94a1u4jAz288pZLtw6yFWVbi89YamiC6JBXPVUj5zmExe5fTVg9 alpha

[reduce_relay]
tx_enable = true
tx_min_peers = 12
tx_relay_percentage = 40
"#
}

fn rpc_only_overlay_text() -> &'static str {
    r#"
[server]
port_rpc

[port_rpc]
ip = 127.0.0.1
port = 5005
protocol = http,ws

[network_id]
testnet

[crawl]
0
"#
}

#[test]
fn build_overlay_setup_parses_config_backed_runtime_defaults_and_overrides() {
    let setup = build_overlay_setup(&config_from_text(configured_overlay_text())).expect("setup");

    assert_eq!(setup.ip_limit, 32);
    assert_eq!(setup.network_id, Some(21_338));
    assert!(setup.tx_reduce_relay_enabled);
    assert_eq!(setup.tx_reduce_relay_min_peers, 12);
    assert_eq!(setup.tx_relay_percentage, 40);
    assert!(setup.client_config.is_some());
    assert!(setup.server_config.is_none());
}

#[test]
fn build_overlay_runtime_carries_server_port_budget_and_secure_listener_tls() {
    let config = config_from_text(configured_overlay_text());
    let mut root = ApplicationRoot::new(0).expect("root");
    root.attach_server_ports_from_config(&config, false)
        .expect("server ports");

    let runtime = build_overlay_runtime(
        &config,
        root.server_ports_setup().as_deref(),
        Arc::new(BootstrapOverlayHandoff),
        None,
        None,
    )
    .expect("runtime");

    let listener = runtime.listener_setup().expect("listener setup");
    assert_eq!(listener.ip, "0.0.0.0");
    assert_eq!(listener.port, 51235);
    assert_eq!(listener.limit, 64);
    assert_eq!(runtime.overlay().limit(), 64);
    assert_eq!(runtime.fd_required(), 128);
    assert_eq!(runtime.network_id(), Some(21_338));
    assert!(runtime.has_listener_tls());
    assert!(!runtime.started());
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        runtime.start().expect("start");
    });
    assert!(runtime.started());
    runtime.stop();
    assert!(runtime.stopped());
    assert!(runtime.overlay().is_stopping());
}

#[test]
fn build_overlay_setup_rejects_too_many_crawl_values_and_bad_public_ip() {
    let too_many_crawl = config_from_text(
        r#"
[crawl]
0
1
"#,
    );
    let invalid_public_ip = config_from_text(
        r#"
[overlay]
public_ip = 127.0.0.1
"#,
    );

    assert_eq!(
        build_overlay_setup(&too_many_crawl)
            .err()
            .expect("too many crawl values"),
        "Configured [crawl] section is invalid, too many values"
    );
    assert_eq!(
        build_overlay_setup(&invalid_public_ip)
            .err()
            .expect("private public ip"),
        "Configured public IP is invalid"
    );
}

#[test]
fn application_root_attach_configured_overlay_runtime_wires_owner_graph() {
    let config = config_from_text(configured_overlay_text());
    let mut root = ApplicationRoot::new(0).expect("root");
    root.attach_server_ports_from_config(&config, false)
        .expect("server ports");
    root.load_cluster_nodes_from_config(&config)
        .expect("cluster nodes");

    let runtime = root
        .attach_configured_overlay_runtime(&config, Arc::new(BootstrapOverlayHandoff))
        .expect("overlay runtime");

    assert!(root.overlay_runtime().is_some());
    assert!(root.overlay_status().is_some());
    assert_eq!(
        ServiceRegistry::get_network_id_service(&root).get_network_id(),
        21_338
    );
    assert_eq!(runtime.network_id(), Some(21_338));

    let reserved = overlay_peer(1, 11);
    let clustered = overlay_peer(2, 12);
    let cluster_key = cluster_public_key();

    runtime.overlay().activate(Arc::clone(&reserved));
    runtime.overlay().activate(Arc::clone(&clustered));

    assert!(
        root.peer_reservations()
            .insert_or_assign(PeerReservation::new(
                reserved.node_public(),
                "bootstrap-reserved",
            ))
            .is_none()
    );
    assert!(root.shared_cluster().update(
        clustered.node_public(),
        "bootstrap-cluster",
        0,
        SystemTime::UNIX_EPOCH + Duration::from_secs(1),
    ));
    runtime.overlay().refresh_membership_state();

    let snapshot = root
        .overlay_status()
        .expect("overlay status")
        .status_snapshot();
    assert_eq!(snapshot.peers, 2);
    assert_eq!(snapshot.network_id, Some(21_338));
    assert!(reserved.reserved());
    assert!(clustered.cluster());
    assert_eq!(
        runtime.overlay().cluster().member(clustered.node_public()),
        Some("bootstrap-cluster".to_owned())
    );
    assert_eq!(
        runtime.overlay().cluster().member(cluster_key),
        Some("alpha".to_owned())
    );
}

#[test]
fn application_root_overlay_runtime_uses_app_owned_reservations_for_limit_bypass_and_refresh() {
    let config = config_from_text(
        r#"
[server]
port_peer

[port_peer]
ip = 0.0.0.0
port = 51235
protocol = peer
limit = 1

[overlay]
ip_limit = 32
"#,
    );
    let mut root = ApplicationRoot::new(0).expect("root");
    root.attach_server_ports_from_config(&config, false)
        .expect("server ports");

    let runtime = root
        .attach_configured_overlay_runtime(&config, Arc::new(BootstrapOverlayHandoff))
        .expect("overlay runtime");
    let first = overlay_peer(3, 13);
    let reserved = overlay_peer(4, 14);

    assert_eq!(runtime.overlay().limit(), 1);
    assert!(runtime.overlay().activate(Arc::clone(&first)));
    assert!(!runtime.overlay().activate(Arc::clone(&reserved)));

    assert!(
        root.peer_reservations()
            .insert_or_assign(PeerReservation::new(
                reserved.node_public(),
                "wallet-backed"
            ))
            .is_none()
    );
    root.wire_overlay_membership_sources(runtime.overlay().as_ref());

    assert!(runtime.overlay().activate(Arc::clone(&reserved)));
    assert_eq!(runtime.overlay().size(), 2);
    assert!(reserved.reserved());

    assert_eq!(
        root.peer_reservations().erase(&reserved.node_public()),
        Some(PeerReservation::new(
            reserved.node_public(),
            "wallet-backed"
        ))
    );
    runtime.overlay().refresh_membership_state();

    assert_eq!(runtime.overlay().size(), 1);
    assert!(
        runtime
            .overlay()
            .find_peer_by_short_id(first.id())
            .is_some()
    );
    assert!(
        runtime
            .overlay()
            .find_peer_by_short_id(reserved.id())
            .is_none()
    );
}

#[test]
fn application_root_cluster_loader_success_and_failure_paths() {
    let valid = config_from_text(
        r#"
[cluster_nodes]
n94a1u4jAz288pZLtw6yFWVbi89YamiC6JBXPVUj5zmExe5fTVg9 alpha
"#,
    );
    let invalid = config_from_text(
        r#"
[cluster_nodes]
not-a-valid-node-public beta
"#,
    );

    let root = ApplicationRoot::new(0).expect("root");
    assert!(
        root.load_cluster_nodes_from_config(&valid)
            .expect("cluster nodes loaded")
    );
    assert_eq!(root.shared_cluster().size(), 1);

    let invalid_root = ApplicationRoot::new(0).expect("root");
    assert_eq!(
        invalid_root
            .load_cluster_nodes_from_config(&invalid)
            .expect_err("invalid cluster nodes"),
        "Invalid entry in cluster configuration."
    );
}

#[test]
fn application_root_cluster_loader_returns_false_when_no_configured_nodes_exist() {
    let config = config_from_text(
        r#"
[server]
port_rpc
"#,
    );
    let root = ApplicationRoot::new(0).expect("root");

    assert!(
        !root
            .load_cluster_nodes_from_config(&config)
            .expect("empty cluster section")
    );
    assert_eq!(root.shared_cluster().size(), 0);
}

#[test]
fn bootstrap_runtime_automatically_owns_overlay_runtime_and_cluster_sources() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = dir.path().join("xrpld.cfg");
    let config = write_config(&config_path, configured_overlay_text());

    let bootstrap = build_bootstrap_runtime(
        &config,
        &AppBootstrapOptions {
            config_path: config_path.clone(),
            standalone: false,
            start_valid: false,
            elb_support: false,
            io_threads: 1,
            job_queue_threads: 1,
            ..AppBootstrapOptions::default()
        },
    )
    .expect("bootstrap");

    assert!(bootstrap.report.has_overlay_runtime);
    assert_eq!(bootstrap.report.overlay_network_id, Some(21_338));
    assert_eq!(bootstrap.report.cluster_node_count, 1);
    assert!(bootstrap.runtime.root().overlay_runtime().is_some());
    assert_eq!(
        ServiceRegistry::get_network_id_service(bootstrap.runtime.root()).get_network_id(),
        21_338
    );
    assert_eq!(
        bootstrap
            .runtime
            .root()
            .overlay_runtime()
            .expect("overlay runtime")
            .overlay()
            .cluster()
            .member(cluster_public_key()),
        Some("alpha".to_owned())
    );
}

#[test]
fn bootstrap_runtime_keeps_overlay_owner_without_a_peer_listener() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = dir.path().join("xrpld.cfg");
    let config = write_config(&config_path, rpc_only_overlay_text());

    let bootstrap = build_bootstrap_runtime(
        &config,
        &AppBootstrapOptions {
            config_path,
            standalone: false,
            start_valid: false,
            elb_support: false,
            io_threads: 1,
            job_queue_threads: 1,
            ..AppBootstrapOptions::default()
        },
    )
    .expect("bootstrap");

    let runtime = bootstrap
        .runtime
        .root()
        .overlay_runtime()
        .expect("overlay runtime");
    let snapshot = bootstrap
        .runtime
        .root()
        .overlay_status()
        .expect("overlay status")
        .status_snapshot();

    assert!(bootstrap.report.has_overlay_runtime);
    assert_eq!(bootstrap.report.overlay_network_id, Some(1));
    assert_eq!(bootstrap.report.cluster_node_count, 0);
    assert!(runtime.listener_setup().is_none());
    assert_eq!(runtime.fd_required(), 0);
    assert_eq!(runtime.network_id(), Some(1));
    assert_eq!(
        ServiceRegistry::get_network_id_service(bootstrap.runtime.root()).get_network_id(),
        1
    );
    assert_eq!(snapshot.network_id, Some(1));
    assert_eq!(snapshot.peers, 0);
}
