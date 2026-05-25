use app::{
    PublishedGrpcPort, PublishedServerPort, ServerPortOverlaySetup, ServerPortsSetup,
    build_server_ports_setup,
};
use basics::basic_config::BasicConfig;

#[test]
fn published_server_port_admin_gate_matches_current_cpp_rule() {
    let unrestricted = PublishedServerPort {
        port: "5005".to_owned(),
        protocols: vec!["http".to_owned(), "ws".to_owned()],
        admin_nets_v4_configured: false,
        admin_nets_v6_configured: false,
        admin_user: None,
        admin_password: None,
    };
    assert!(!unrestricted.has_admin_restrictions());

    let admin_net = PublishedServerPort {
        admin_nets_v4_configured: true,
        ..unrestricted.clone()
    };
    assert!(admin_net.has_admin_restrictions());

    let admin_user = PublishedServerPort {
        admin_user: Some("rpc".to_owned()),
        ..unrestricted
    };
    assert!(admin_user.has_admin_restrictions());
}

#[test]
fn published_grpc_port_keeps_ip_and_port_as_strings() {
    let grpc = PublishedGrpcPort {
        ip: "127.0.0.1".to_owned(),
        port: "50051".to_owned(),
    };

    assert_eq!(grpc.ip, "127.0.0.1");
    assert_eq!(grpc.port, "50051");
}

#[test]
fn build_server_ports_setup_derives_client_overlay_and_grpc() {
    let mut config = BasicConfig::new();
    let server = config.section_mut("server");
    server.set("protocol", "http");
    server.set("admin_user", "rpc");
    server.append("port_rpc");
    server.append("port_peer");
    server.append("port_grpc");

    let rpc = config.section_mut("port_rpc");
    rpc.set("ip", "0.0.0.0");
    rpc.set("port", "5005");
    rpc.set("user", "rpc-user");
    rpc.set("password", "secret");

    let peer = config.section_mut("port_peer");
    peer.set("ip", "127.0.0.1");
    peer.set("port", "51235");
    peer.set("protocol", "peer");

    let grpc = config.section_mut("port_grpc");
    grpc.set("ip", "127.0.0.1");
    grpc.set("port", "50051");

    let setup = build_server_ports_setup(&config, false).expect("setup should parse");

    assert_eq!(setup.ports.len(), 2);
    assert_eq!(setup.client.as_ref().expect("client setup").ip, "127.0.0.1");
    assert!(!setup.client.as_ref().expect("client setup").secure);
    assert_eq!(
        setup.overlay,
        Some(ServerPortOverlaySetup {
            ip: "127.0.0.1".to_owned(),
            port: 51235,
            limit: 0,
            secure: true,
            ssl_key: String::new(),
            ssl_cert: String::new(),
            ssl_chain: String::new(),
            ssl_ciphers: String::new(),
        })
    );
    assert_eq!(
        setup.grpc,
        Some(PublishedGrpcPort {
            ip: "127.0.0.1".to_owned(),
            port: "50051".to_owned(),
        })
    );
    assert_eq!(
        setup.published_server_ports()[0].admin_user,
        Some("rpc".to_owned())
    );
    assert_eq!(setup.fd_required(), 1024);
}

#[test]
fn build_server_ports_setup_strips_peer_and_empty_ports_in_standalone() {
    let mut config = BasicConfig::new();
    config.section_mut("server").append("port_peer");
    config.section_mut("server").append("port_ws");

    let peer = config.section_mut("port_peer");
    peer.set("ip", "127.0.0.1");
    peer.set("port", "51235");
    peer.set("protocol", "peer");

    let ws = config.section_mut("port_ws");
    ws.set("ip", "127.0.0.1");
    ws.set("port", "5005");
    ws.set("protocol", "peer,ws");

    let setup = ServerPortsSetup::from_config(&config, true).expect("setup should parse");

    assert_eq!(setup.ports.len(), 1);
    assert_eq!(setup.ports[0].name, "port_ws");
    assert_eq!(setup.ports[0].protocols, vec!["ws".to_owned()]);
    assert!(setup.overlay.is_none());
}

#[test]
fn build_server_ports_setup_rejects_multiple_peer_ports() {
    let mut config = BasicConfig::new();
    config.section_mut("server").append("port_peer_1");
    config.section_mut("server").append("port_peer_2");

    let first = config.section_mut("port_peer_1");
    first.set("ip", "127.0.0.1");
    first.set("port", "51235");
    first.set("protocol", "peer");

    let second = config.section_mut("port_peer_2");
    second.set("ip", "127.0.0.1");
    second.set("port", "61235");
    second.set("protocol", "peer");

    assert_eq!(
        build_server_ports_setup(&config, false),
        Err("Error: More than one peer protocol configured in [server]".to_owned())
    );
}

#[test]
fn build_server_ports_setup_rejects_zero_named_port() {
    let mut config = BasicConfig::new();
    config.section_mut("server").append("port_rpc");
    let rpc = config.section_mut("port_rpc");
    rpc.set("ip", "127.0.0.1");
    rpc.set("port", "0");
    rpc.set("protocol", "http");

    assert_eq!(
        build_server_ports_setup(&config, false),
        Err("Invalid value '0' for key 'port' in [port_rpc]".to_owned())
    );
}
