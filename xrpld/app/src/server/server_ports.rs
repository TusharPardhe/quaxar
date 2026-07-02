//! Narrow server-port setup seam for `server_info` / `server_state`.
//!
//! This keeps the real setup inputs in app-owned state so later ports can
//! consume config-derived client and overlay data without forcing the RPC
//! shaper to invent that ownership.

use basics::basic_config::BasicConfig;
use ipnet::IpNet;
use std::net::IpAddr;
use xrpld_core::{
    ParsedServerPortConfig, parse_grpc_port_config, parse_server_port_configs,
    validate_zero_port_server_sections,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedServerPort {
    pub port: String,
    pub protocols: Vec<String>,
    pub admin_nets_v4_configured: bool,
    pub admin_nets_v6_configured: bool,
    pub admin_user: Option<String>,
    pub admin_password: Option<String>,
}

impl PublishedServerPort {
    pub fn has_admin_restrictions(&self) -> bool {
        self.admin_nets_v4_configured
            || self.admin_nets_v6_configured
            || self
                .admin_user
                .as_ref()
                .is_some_and(|value| !value.is_empty())
            || self
                .admin_password
                .as_ref()
                .is_some_and(|value| !value.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedGrpcPort {
    pub ip: String,
    pub port: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerPortClientSetup {
    pub secure: bool,
    pub ip: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub admin_user: String,
    pub admin_password: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerPortOverlaySetup {
    pub ip: String,
    pub port: u16,
    pub limit: u32,
    pub secure: bool,
    pub ssl_key: String,
    pub ssl_cert: String,
    pub ssl_chain: String,
    pub ssl_ciphers: String,
}

impl ServerPortOverlaySetup {
    pub fn fd_required(&self) -> usize {
        // Match `ApplicationImp::fdRequired()` in the reference reference: the
        // overlay budget is exactly two descriptors per configured peer
        // connection limit, with no extra floor when the limit is zero.
        self.limit.saturating_mul(2) as usize
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerPortSetup {
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub limit: u32,
    pub protocols: Vec<String>,
    pub user: String,
    pub password: String,
    pub admin_user: String,
    pub admin_password: String,
    pub ssl_key: String,
    pub ssl_cert: String,
    pub ssl_chain: String,
    pub ssl_ciphers: String,
    pub admin_nets_v4: Vec<IpNet>,
    pub admin_nets_v6: Vec<IpNet>,
    pub secure_gateway_nets_v4: Vec<IpNet>,
    pub secure_gateway_nets_v6: Vec<IpNet>,
    pub standalone_mode: bool,
}

impl ServerPortSetup {
    fn from_parsed_config(parsed: ParsedServerPortConfig, limit: u32) -> Self {
        Self {
            name: parsed.name,
            ip: parsed.ip.to_string(),
            port: parsed.port,
            limit,
            protocols: parsed.protocols,
            user: parsed.user,
            password: parsed.password,
            admin_user: parsed.admin_user,
            admin_password: parsed.admin_password,
            ssl_key: parsed.ssl_key,
            ssl_cert: parsed.ssl_cert,
            ssl_chain: parsed.ssl_chain,
            ssl_ciphers: parsed.ssl_ciphers,
            admin_nets_v4: parsed.admin_nets_v4,
            admin_nets_v6: parsed.admin_nets_v6,
            secure_gateway_nets_v4: parsed.secure_gateway_nets_v4,
            secure_gateway_nets_v6: parsed.secure_gateway_nets_v6,
            standalone_mode: false,
        }
    }

    pub fn has_protocol(&self, protocol: &str) -> bool {
        self.protocols.iter().any(|candidate| candidate == protocol)
    }

    pub fn allows_http(&self) -> bool {
        self.has_protocol("http") || self.has_protocol("https")
    }

    pub fn allows_websocket(&self) -> bool {
        self.has_protocol("ws")
            || self.has_protocol("ws2")
            || self.has_protocol("wss")
            || self.has_protocol("wss2")
    }

    pub fn allows_peer(&self) -> bool {
        self.has_protocol("peer")
    }

    pub fn secure(&self) -> bool {
        self.has_protocol("peer")
            || self.has_protocol("https")
            || self.has_protocol("wss")
            || self.has_protocol("wss2")
    }

    pub fn fd_required(&self) -> usize {
        self.limit.max(256) as usize
    }

    pub fn published(&self) -> PublishedServerPort {
        PublishedServerPort {
            port: self.port.to_string(),
            protocols: self.protocols.clone(),
            admin_nets_v4_configured: !self.admin_nets_v4.is_empty(),
            admin_nets_v6_configured: !self.admin_nets_v6.is_empty(),
            admin_user: if self.admin_user.is_empty() {
                None
            } else {
                Some(self.admin_user.clone())
            },
            admin_password: if self.admin_password.is_empty() {
                None
            } else {
                Some(self.admin_password.clone())
            },
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerPortsSetup {
    pub ports: Vec<ServerPortSetup>,
    pub client: Option<ServerPortClientSetup>,
    pub overlay: Option<ServerPortOverlaySetup>,
    pub grpc: Option<PublishedGrpcPort>,
}

impl ServerPortsSetup {
    pub fn from_config(config: &BasicConfig, stand_alone: bool) -> Result<Self, String> {
        validate_zero_port_server_sections(config)?;

        let ports = parse_server_port_configs(config, stand_alone)?
            .into_iter()
            .map(|parsed| {
                let limit = config
                    .section(parsed.name.as_str())
                    .get::<u32>("limit")
                    .ok()
                    .flatten()
                    .or_else(|| config.section("server").get::<u32>("limit").ok().flatten())
                    .unwrap_or(0);
                let mut port = ServerPortSetup::from_parsed_config(parsed, limit);
                port.standalone_mode = stand_alone;
                port
            })
            .collect::<Vec<_>>();
        let grpc = parse_grpc_port_config(config).map(|grpc| PublishedGrpcPort {
            ip: grpc.ip,
            port: grpc.port,
        });

        let mut setup = Self {
            ports,
            client: None,
            overlay: None,
            grpc,
        };
        setup.client = setup.derive_client();
        setup.overlay = setup.derive_overlay();
        Ok(setup)
    }

    pub fn published_server_ports(&self) -> Vec<PublishedServerPort> {
        self.ports.iter().map(ServerPortSetup::published).collect()
    }

    pub fn published_grpc_port(&self) -> Option<PublishedGrpcPort> {
        self.grpc.clone()
    }

    pub fn fd_required(&self) -> usize {
        let needed = self
            .ports
            .iter()
            .map(ServerPortSetup::fd_required)
            .sum::<usize>()
            + self
                .overlay
                .as_ref()
                .map_or(0, ServerPortOverlaySetup::fd_required);
        needed.max(1024)
    }

    fn derive_client(&self) -> Option<ServerPortClientSetup> {
        let port = self
            .ports
            .iter()
            .find(|port| port.has_protocol("http") || port.has_protocol("https"))?;

        Some(ServerPortClientSetup {
            secure: port.has_protocol("https"),
            ip: localhost_client_ip(&port.ip),
            port: port.port,
            user: port.user.clone(),
            password: port.password.clone(),
            admin_user: port.admin_user.clone(),
            admin_password: port.admin_password.clone(),
        })
    }

    fn derive_overlay(&self) -> Option<ServerPortOverlaySetup> {
        let port = self.ports.iter().find(|port| port.has_protocol("peer"))?;
        Some(ServerPortOverlaySetup {
            ip: port.ip.clone(),
            port: port.port,
            limit: port.limit,
            secure: port.secure(),
            ssl_key: port.ssl_key.clone(),
            ssl_cert: port.ssl_cert.clone(),
            ssl_chain: port.ssl_chain.clone(),
            ssl_ciphers: port.ssl_ciphers.clone(),
        })
    }
}

pub fn build_server_ports_setup(
    config: &BasicConfig,
    stand_alone: bool,
) -> Result<ServerPortsSetup, String> {
    ServerPortsSetup::from_config(config, stand_alone)
}

fn localhost_client_ip(ip: &str) -> String {
    match ip.parse::<IpAddr>() {
        Ok(IpAddr::V4(addr)) if addr.is_unspecified() => "127.0.0.1".to_owned(),
        Ok(IpAddr::V6(addr)) if addr.is_unspecified() => "::1".to_owned(),
        _ => ip.to_owned(),
    }
}

pub trait PublishedServerPortsSource: Send + Sync + 'static {
    fn published_server_ports(&self) -> Vec<PublishedServerPort>;

    fn published_grpc_port(&self) -> Option<PublishedGrpcPort> {
        None
    }
}

impl PublishedServerPortsSource for ServerPortsSetup {
    fn published_server_ports(&self) -> Vec<PublishedServerPort> {
        self.published_server_ports()
    }

    fn published_grpc_port(&self) -> Option<PublishedGrpcPort> {
        self.published_grpc_port()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PublishedGrpcPort, PublishedServerPort, ServerPortClientSetup, ServerPortOverlaySetup,
        ServerPortSetup, ServerPortsSetup, localhost_client_ip,
    };
    use basics::basic_config::BasicConfig;

    #[test]
    fn published_server_port_detects_admin_restrictions() {
        let unrestricted = PublishedServerPort {
            port: "5005".to_owned(),
            protocols: vec!["http".to_owned()],
            admin_nets_v4_configured: false,
            admin_nets_v6_configured: false,
            admin_user: None,
            admin_password: None,
        };
        assert!(!unrestricted.has_admin_restrictions());

        let restricted = PublishedServerPort {
            admin_password: Some("secret".to_owned()),
            ..unrestricted
        };
        assert!(restricted.has_admin_restrictions());
    }

    #[test]
    fn server_ports_setup_projects_published_view_without_losing_setup_data() {
        let setup = ServerPortsSetup {
            ports: vec![ServerPortSetup {
                name: "port_rpc".to_owned(),
                ip: "127.0.0.1".to_owned(),
                port: 5005,
                limit: 0,
                protocols: vec!["http".to_owned(), "ws2".to_owned()],
                user: "rpc".to_owned(),
                password: "secret".to_owned(),
                admin_user: "admin".to_owned(),
                admin_password: "pw".to_owned(),
                ssl_key: "server.key".to_owned(),
                ssl_cert: "server.crt".to_owned(),
                ssl_chain: "chain.pem".to_owned(),
                ssl_ciphers: "HIGH:!aNULL".to_owned(),
                admin_nets_v4: vec!["127.0.0.0/8".parse().expect("admin net should parse")],
                admin_nets_v6: Vec::new(),
                secure_gateway_nets_v4: vec![
                    "10.0.0.0/8"
                        .parse()
                        .expect("secure gateway net should parse"),
                ],
                secure_gateway_nets_v6: Vec::new(),
                standalone_mode: false,
            }],
            client: Some(ServerPortClientSetup {
                secure: false,
                ip: "127.0.0.1".to_owned(),
                port: 5005,
                user: "rpc".to_owned(),
                password: "secret".to_owned(),
                admin_user: "admin".to_owned(),
                admin_password: "pw".to_owned(),
            }),
            overlay: Some(ServerPortOverlaySetup {
                ip: "127.0.0.1".to_owned(),
                port: 51235,
                limit: 0,
                secure: true,
                ssl_key: String::new(),
                ssl_cert: String::new(),
                ssl_chain: String::new(),
                ssl_ciphers: String::new(),
            }),
            grpc: Some(PublishedGrpcPort {
                ip: "127.0.0.1".to_owned(),
                port: "50051".to_owned(),
            }),
        };

        assert_eq!(
            setup.published_server_ports(),
            vec![PublishedServerPort {
                port: "5005".to_owned(),
                protocols: vec!["http".to_owned(), "ws2".to_owned()],
                admin_nets_v4_configured: true,
                admin_nets_v6_configured: false,
                admin_user: Some("admin".to_owned()),
                admin_password: Some("pw".to_owned()),
            }]
        );
        assert_eq!(setup.published_grpc_port(), setup.grpc);
        assert_eq!(setup.client.as_ref().map(|client| client.port), Some(5005));
        assert_eq!(
            setup.overlay.as_ref().map(|overlay| overlay.port),
            Some(51235)
        );
        assert_eq!(setup.fd_required(), 1024);
    }

    #[test]
    fn server_port_overlay_fd_required_peer_budget_rule() {
        assert_eq!(
            ServerPortOverlaySetup {
                ip: "127.0.0.1".to_owned(),
                port: 51235,
                limit: 0,
                secure: false,
                ssl_key: String::new(),
                ssl_cert: String::new(),
                ssl_chain: String::new(),
                ssl_ciphers: String::new(),
            }
            .fd_required(),
            0
        );
        assert_eq!(
            ServerPortOverlaySetup {
                ip: "127.0.0.1".to_owned(),
                port: 51235,
                limit: 600,
                secure: true,
                ssl_key: String::new(),
                ssl_cert: String::new(),
                ssl_chain: String::new(),
                ssl_ciphers: String::new(),
            }
            .fd_required(),
            1200
        );
    }

    #[test]
    fn server_ports_setup_derives_client_localhost_for_unspecified_addresses() {
        assert_eq!(localhost_client_ip("0.0.0.0"), "127.0.0.1");
        assert_eq!(localhost_client_ip("::"), "::1");
        assert_eq!(localhost_client_ip("127.0.0.1"), "127.0.0.1");
    }

    #[test]
    fn server_port_setup_tracks_runtime_protocol_capabilities() {
        let port = ServerPortSetup {
            name: "port_rpc".to_owned(),
            ip: "127.0.0.1".to_owned(),
            port: 5005,
            limit: 0,
            protocols: vec!["https".to_owned(), "wss".to_owned(), "peer".to_owned()],
            user: "rpc".to_owned(),
            password: "secret".to_owned(),
            admin_user: String::new(),
            admin_password: String::new(),
            ssl_key: String::new(),
            ssl_cert: String::new(),
            ssl_chain: String::new(),
            ssl_ciphers: String::new(),
            admin_nets_v4: Vec::new(),
            admin_nets_v6: Vec::new(),
            secure_gateway_nets_v4: Vec::new(),
            secure_gateway_nets_v6: Vec::new(),
            standalone_mode: false,
        };

        assert!(port.allows_http());
        assert!(port.allows_websocket());
        assert!(port.allows_peer());
        assert!(port.secure());
        assert_eq!(port.fd_required(), 256);
    }

    #[test]
    fn server_ports_setup_builds_runtime_setup_from_config() {
        let mut config = BasicConfig::new();
        let server = config.section_mut("server");
        server.set("protocol", "http");
        server.set("admin_user", "rpc");
        server.append("port_rpc");
        server.append("port_peer");
        server.append("port_grpc");

        let port_rpc = config.section_mut("port_rpc");
        port_rpc.set("ip", "::");
        port_rpc.set("port", "5005");
        port_rpc.set("user", "user");
        port_rpc.set("password", "pw");

        let port_peer = config.section_mut("port_peer");
        port_peer.set("ip", "127.0.0.1");
        port_peer.set("port", "51235");
        port_peer.set("protocol", "peer");
        port_peer.set("ssl_key", "peer.key");
        port_peer.set("ssl_cert", "peer.crt");
        port_peer.set("ssl_chain", "peer.chain");
        port_peer.set("ssl_ciphers", "ECDHE+AESGCM");

        let port_grpc = config.section_mut("port_grpc");
        port_grpc.set("ip", "127.0.0.1");
        port_grpc.set("port", "50051");

        let setup = ServerPortsSetup::from_config(&config, false).expect("setup should parse");

        assert_eq!(setup.ports.len(), 2);
        assert_eq!(setup.client.as_ref().expect("client setup").ip, "::1");
        assert!(setup.client.as_ref().expect("client setup").admin_user == "rpc");
        assert_eq!(
            setup.overlay,
            Some(ServerPortOverlaySetup {
                ip: "127.0.0.1".to_owned(),
                port: 51235,
                limit: 0,
                secure: true,
                ssl_key: "peer.key".to_owned(),
                ssl_cert: "peer.crt".to_owned(),
                ssl_chain: "peer.chain".to_owned(),
                ssl_ciphers: "ECDHE+AESGCM".to_owned(),
            })
        );
        assert_eq!(
            setup.grpc,
            Some(PublishedGrpcPort {
                ip: "127.0.0.1".to_owned(),
                port: "50051".to_owned(),
            })
        );
        assert_eq!(setup.fd_required(), 1024);
    }

    #[test]
    fn server_ports_setup_fd_required_accounts_for_configured_limits() {
        let mut config = BasicConfig::new();
        let server = config.section_mut("server");
        server.append("port_peer");

        let port_peer = config.section_mut("port_peer");
        port_peer.set("ip", "127.0.0.1");
        port_peer.set("port", "51235");
        port_peer.set("protocol", "peer");
        port_peer.set("limit", "600");

        let setup = ServerPortsSetup::from_config(&config, false).expect("setup should parse");

        assert_eq!(setup.ports[0].limit, 600);
        assert_eq!(setup.overlay.as_ref().expect("overlay setup").limit, 600);
        assert_eq!(setup.fd_required(), 1800);
    }
}
