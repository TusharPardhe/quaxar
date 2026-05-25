use basics::basic_config::{BasicConfig, Section};
use ipnet::IpNet;
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub const SERVER_SECTION: &str = "server";
pub const GRPC_SERVER_PORT_SECTION: &str = "port_grpc";
const PEER_PROTOCOL: &str = "peer";
type ParsedNetworkLists = (Vec<IpNet>, Vec<IpNet>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedServerPortConfig {
    pub name: String,
    pub ip: IpAddr,
    pub port: u16,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedGrpcPortConfig {
    pub ip: String,
    pub port: String,
}

#[derive(Debug, Clone, Default)]
struct PartialServerPortConfig {
    ip: Option<IpAddr>,
    port: Option<u16>,
    protocols: BTreeSet<String>,
    user: String,
    password: String,
    admin_user: String,
    admin_password: String,
    ssl_key: String,
    ssl_cert: String,
    ssl_chain: String,
    ssl_ciphers: String,
    admin_nets_v4: Vec<IpNet>,
    admin_nets_v6: Vec<IpNet>,
    secure_gateway_nets_v4: Vec<IpNet>,
    secure_gateway_nets_v6: Vec<IpNet>,
}

pub fn validate_zero_port_server_sections(config: &BasicConfig) -> Result<(), String> {
    if !config.exists(SERVER_SECTION) {
        return Ok(());
    }

    for name in config.section(SERVER_SECTION).values() {
        if !config.exists(name) {
            // Match the reference loader: the first missing named section aborts the
            // zero-port scan without reporting a validation error.
            return Ok(());
        }

        let section = config.section(name);
        if let Ok(Some(raw_port)) = section.get::<String>("port") {
            let port = raw_port
                .parse::<u16>()
                .map_err(|_| invalid_port_message(name, &raw_port))?;
            if port == 0 {
                return Err(invalid_port_message(name, &raw_port));
            }
        }
    }

    Ok(())
}

pub fn parse_server_port_configs(
    config: &BasicConfig,
    stand_alone: bool,
) -> Result<Vec<ParsedServerPortConfig>, String> {
    if !config.exists(SERVER_SECTION) {
        return Err("Required section [server] is missing".to_owned());
    }

    let mut common = PartialServerPortConfig::default();
    merge_section(&mut common, config.section(SERVER_SECTION))?;

    let mut result = Vec::new();
    for name in config.section(SERVER_SECTION).values() {
        if !config.exists(name) {
            return Err(format!("Missing section: [{name}]"));
        }

        if name == GRPC_SERVER_PORT_SECTION {
            continue;
        }

        let mut merged = common.clone();
        merge_section(&mut merged, config.section(name))?;
        let mut parsed = finalize_section(name, merged)?;

        if stand_alone {
            parsed
                .protocols
                .retain(|protocol| protocol != PEER_PROTOCOL);
            if parsed.protocols.is_empty() {
                continue;
            }
        }

        result.push(parsed);
    }

    if !stand_alone {
        let peer_ports = result
            .iter()
            .filter(|port| {
                port.protocols
                    .iter()
                    .any(|protocol| protocol == PEER_PROTOCOL)
            })
            .count();
        if peer_ports > 1 {
            return Err("Error: More than one peer protocol configured in [server]".to_owned());
        }
    }

    Ok(result)
}

pub fn parse_grpc_port_config(config: &BasicConfig) -> Option<ParsedGrpcPortConfig> {
    let section = config.section(GRPC_SERVER_PORT_SECTION);
    let ip = section.get::<String>("ip").ok().flatten()?;
    let port = section.get::<String>("port").ok().flatten()?;
    Some(ParsedGrpcPortConfig { ip, port })
}

fn merge_section(target: &mut PartialServerPortConfig, section: &Section) -> Result<(), String> {
    if let Ok(Some(raw_ip)) = section.get::<String>("ip") {
        target.ip = Some(
            raw_ip
                .parse::<IpAddr>()
                .map_err(|_| invalid_ip_message(section.name(), &raw_ip))?,
        );
    }

    if let Ok(Some(raw_port)) = section.get::<String>("port") {
        let port = raw_port
            .parse::<u16>()
            .map_err(|_| invalid_port_message(section.name(), &raw_port))?;
        if port == 0 && section.name() == SERVER_SECTION {
            return Err(invalid_port_message(section.name(), &raw_port));
        }
        target.port = Some(port);
    }

    if let Ok(Some(raw_protocols)) = section.get::<String>("protocol") {
        for protocol in raw_protocols.split(',') {
            let protocol = protocol.trim().to_ascii_lowercase();
            if !protocol.is_empty() {
                target.protocols.insert(protocol);
            }
        }
    }

    if let Ok(Some(user)) = section.get::<String>("user") {
        target.user = user;
    }
    if let Ok(Some(password)) = section.get::<String>("password") {
        target.password = password;
    }
    if let Ok(Some(admin_user)) = section.get::<String>("admin_user") {
        target.admin_user = admin_user;
    }
    if let Ok(Some(admin_password)) = section.get::<String>("admin_password") {
        target.admin_password = admin_password;
    }
    if let Ok(Some(ssl_key)) = section.get::<String>("ssl_key") {
        target.ssl_key = ssl_key;
    }
    if let Ok(Some(ssl_cert)) = section.get::<String>("ssl_cert") {
        target.ssl_cert = ssl_cert;
    }
    if let Ok(Some(ssl_chain)) = section.get::<String>("ssl_chain") {
        target.ssl_chain = ssl_chain;
    }
    if let Ok(Some(ssl_ciphers)) = section.get::<String>("ssl_ciphers") {
        target.ssl_ciphers = ssl_ciphers;
    }

    if let Some((nets_v4, nets_v6)) = parse_networks(section, "admin")? {
        target.admin_nets_v4.extend(nets_v4);
        target.admin_nets_v6.extend(nets_v6);
    }
    if let Some((nets_v4, nets_v6)) = parse_networks(section, "secure_gateway")? {
        target.secure_gateway_nets_v4.extend(nets_v4);
        target.secure_gateway_nets_v6.extend(nets_v6);
    }

    Ok(())
}

fn finalize_section(
    name: &str,
    merged: PartialServerPortConfig,
) -> Result<ParsedServerPortConfig, String> {
    let ip = merged
        .ip
        .ok_or_else(|| format!("Missing 'ip' in [{name}]"))?;
    let port = merged
        .port
        .ok_or_else(|| format!("Missing 'port' in [{name}]"))?;
    if merged.protocols.is_empty() {
        return Err(format!("Missing 'protocol' in [{name}]"));
    }

    Ok(ParsedServerPortConfig {
        name: name.to_owned(),
        ip,
        port,
        protocols: merged.protocols.into_iter().collect(),
        user: merged.user,
        password: merged.password,
        admin_user: merged.admin_user,
        admin_password: merged.admin_password,
        ssl_key: merged.ssl_key,
        ssl_cert: merged.ssl_cert,
        ssl_chain: merged.ssl_chain,
        ssl_ciphers: merged.ssl_ciphers,
        admin_nets_v4: merged.admin_nets_v4,
        admin_nets_v6: merged.admin_nets_v6,
        secure_gateway_nets_v4: merged.secure_gateway_nets_v4,
        secure_gateway_nets_v6: merged.secure_gateway_nets_v6,
    })
}

fn parse_networks(section: &Section, field: &str) -> Result<Option<ParsedNetworkLists>, String> {
    let Ok(Some(raw_value)) = section.get::<String>(field) else {
        return Ok(None);
    };

    let mut nets_v4 = Vec::new();
    let mut nets_v6 = Vec::new();

    for candidate in raw_value.split(',') {
        let candidate = candidate.trim();
        if candidate.is_empty() {
            continue;
        }

        if let Ok(ip) = candidate.parse::<IpAddr>() {
            if ip.is_unspecified() {
                nets_v4.push(IpNet::from(IpAddr::V4(Ipv4Addr::UNSPECIFIED)).trunc());
                nets_v6.push(IpNet::from(IpAddr::V6(Ipv6Addr::UNSPECIFIED)).trunc());
                break;
            }

            match ip {
                IpAddr::V4(ip) => nets_v4.push(IpNet::new(ip.into(), 32).expect("valid /32")),
                IpAddr::V6(ip) => nets_v6.push(IpNet::new(ip.into(), 128).expect("valid /128")),
            }
            continue;
        }

        let network = candidate
            .parse::<IpNet>()
            .map_err(|_| invalid_network_message(section.name(), field, candidate))?;
        let canonical = network.trunc();
        if network != canonical {
            return Err(format!(
                "The configured subnet {network} is not the same as the network address, which is {canonical}"
            ));
        }

        match network {
            IpNet::V4(network) => nets_v4.push(IpNet::V4(network)),
            IpNet::V6(network) => nets_v6.push(IpNet::V6(network)),
        }
    }

    Ok(Some((nets_v4, nets_v6)))
}

fn invalid_ip_message(section: &str, value: &str) -> String {
    format!("Invalid value '{value}' for key 'ip' in [{section}]")
}

fn invalid_port_message(section: &str, value: &str) -> String {
    format!("Invalid value '{value}' for key 'port' in [{section}]")
}

fn invalid_network_message(section: &str, field: &str, value: &str) -> String {
    format!("Invalid value '{value}' for key '{field}' in [{section}]")
}

#[cfg(test)]
mod tests {
    use super::{
        GRPC_SERVER_PORT_SECTION, SERVER_SECTION, parse_grpc_port_config,
        parse_server_port_configs, validate_zero_port_server_sections,
    };
    use basics::basic_config::BasicConfig;
    use ipnet::IpNet;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn validate_zero_ports_rejects_named_server_port_zero() {
        let mut config = BasicConfig::new();
        config.section_mut(SERVER_SECTION).append("port_rpc");
        config.section_mut("port_rpc").set("port", "0");

        assert_eq!(
            validate_zero_port_server_sections(&config),
            Err("Invalid value '0' for key 'port' in [port_rpc]".to_owned())
        );
    }

    #[test]
    fn validate_zero_ports_stops_at_first_missing_section() {
        let mut config = BasicConfig::new();
        config.section_mut(SERVER_SECTION).append("missing_port");
        config.section_mut(SERVER_SECTION).append("port_rpc");
        config.section_mut("port_rpc").set("port", "0");

        assert_eq!(validate_zero_port_server_sections(&config), Ok(()));
    }

    #[test]
    fn validate_zero_ports_still_rejects_present_zero_port_sections() {
        let mut config = BasicConfig::new();
        config.section_mut(SERVER_SECTION).append("port_rpc");
        config.section_mut("port_rpc").set("port", "0");

        assert_eq!(
            validate_zero_port_server_sections(&config),
            Err("Invalid value '0' for key 'port' in [port_rpc]".to_owned())
        );
    }

    #[test]
    fn parse_ports_requires_server_section_and_named_sections() {
        let config = BasicConfig::new();
        assert_eq!(
            parse_server_port_configs(&config, false),
            Err("Required section [server] is missing".to_owned())
        );

        let mut missing_named = BasicConfig::new();
        missing_named.section_mut(SERVER_SECTION).append("port_rpc");
        assert_eq!(
            parse_server_port_configs(&missing_named, false),
            Err("Missing section: [port_rpc]".to_owned())
        );
    }

    #[test]
    fn parse_ports_merge_common_values_and_enforce_single_peer_rule() {
        let mut config = BasicConfig::new();
        let server = config.section_mut(SERVER_SECTION);
        server.set("protocol", "http");
        server.set("admin_user", "rpc");
        server.set("ssl_ciphers", "HIGH:!aNULL");
        server.append("port_rpc");
        server.append("port_peer");

        let port_rpc = config.section_mut("port_rpc");
        port_rpc.set("ip", "127.0.0.1");
        port_rpc.set("port", "5005");

        let port_peer = config.section_mut("port_peer");
        port_peer.set("ip", "127.0.0.1");
        port_peer.set("port", "51235");
        port_peer.set("protocol", "peer");

        let ports = parse_server_port_configs(&config, false).expect("ports should parse");
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].protocols, vec!["http".to_owned()]);
        assert_eq!(
            ports[1].protocols,
            vec!["http".to_owned(), "peer".to_owned()]
        );
        assert_eq!(ports[0].admin_user, "rpc");
        assert_eq!(ports[0].ssl_ciphers, "HIGH:!aNULL");

        config.section_mut("port_ws").set("ip", "127.0.0.1");
        config.section_mut("port_ws").set("port", "6006");
        config.section_mut("port_ws").set("protocol", "peer");
        config.section_mut(SERVER_SECTION).append("port_ws");

        assert_eq!(
            parse_server_port_configs(&config, false),
            Err("Error: More than one peer protocol configured in [server]".to_owned())
        );
    }

    #[test]
    fn parse_ports_strip_peer_and_remove_empty_standalone_ports() {
        let mut config = BasicConfig::new();
        config.section_mut(SERVER_SECTION).append("port_peer");
        config.section_mut(SERVER_SECTION).append("port_mixed");

        let peer = config.section_mut("port_peer");
        peer.set("ip", "127.0.0.1");
        peer.set("port", "51235");
        peer.set("protocol", "peer");

        let mixed = config.section_mut("port_mixed");
        mixed.set("ip", "127.0.0.1");
        mixed.set("port", "5005");
        mixed.set("protocol", "peer,ws");

        let ports = parse_server_port_configs(&config, true).expect("ports should parse");
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].name, "port_mixed");
        assert_eq!(ports[0].protocols, vec!["ws".to_owned()]);
    }

    #[test]
    fn parse_ports_parse_admin_and_secure_gateway_networks() {
        let mut config = BasicConfig::new();
        config.section_mut(SERVER_SECTION).append("port_rpc");

        let port = config.section_mut("port_rpc");
        port.set("ip", "::");
        port.set("port", "5005");
        port.set("protocol", "https");
        port.set("admin", "0.0.0.0");
        port.set("secure_gateway", "192.168.1.10,2001:db8::/64");

        let ports = parse_server_port_configs(&config, false).expect("ports should parse");
        assert_eq!(ports[0].ip, IpAddr::V6(Ipv6Addr::UNSPECIFIED));
        assert_eq!(
            ports[0].admin_nets_v4,
            vec![IpNet::from(IpAddr::V4(Ipv4Addr::UNSPECIFIED)).trunc()]
        );
        assert_eq!(
            ports[0].admin_nets_v6,
            vec![IpNet::from(IpAddr::V6(Ipv6Addr::UNSPECIFIED)).trunc()]
        );
        assert_eq!(
            ports[0].secure_gateway_nets_v4,
            vec![IpNet::new(Ipv4Addr::new(192, 168, 1, 10).into(), 32).expect("valid net")]
        );
        assert_eq!(
            ports[0].secure_gateway_nets_v6,
            vec!["2001:db8::/64".parse::<IpNet>().expect("valid net")]
        );
    }

    #[test]
    fn parse_ports_keep_ssl_material_fields() {
        let mut config = BasicConfig::new();
        config.section_mut(SERVER_SECTION).append("port_peer");

        let port = config.section_mut("port_peer");
        port.set("ip", "127.0.0.1");
        port.set("port", "51235");
        port.set("protocol", "peer");
        port.set("ssl_key", "server.key");
        port.set("ssl_cert", "server.crt");
        port.set("ssl_chain", "chain.pem");
        port.set("ssl_ciphers", "ECDHE+AESGCM");

        let ports = parse_server_port_configs(&config, false).expect("ports should parse");
        assert_eq!(ports[0].ssl_key, "server.key");
        assert_eq!(ports[0].ssl_cert, "server.crt");
        assert_eq!(ports[0].ssl_chain, "chain.pem");
        assert_eq!(ports[0].ssl_ciphers, "ECDHE+AESGCM");
    }

    #[test]
    fn parse_grpc_reads_raw_config_strings_without_validating_them() {
        let mut config = BasicConfig::new();
        let grpc = config.section_mut(GRPC_SERVER_PORT_SECTION);
        grpc.set("ip", "0.0.0.0");
        grpc.set("port", "50051");

        let parsed = parse_grpc_port_config(&config).expect("grpc config should parse");
        assert_eq!(parsed.ip, "0.0.0.0");
        assert_eq!(parsed.port, "50051");
    }
}
