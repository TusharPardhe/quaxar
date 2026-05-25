use std::net::SocketAddr;
use std::sync::Arc;
use std::thread;

use app::{ApplicationRoot, ApplicationRootOptions, ServerPortSetup, ServerPortsSetup};
use rpc::{ApplicationServerInfo, OwnedApplicationServerInfo};

use crate::{BuiltinDispatcher, RpcServerPortDeferredProtocol, ServerRuntime};

#[derive(Debug, Clone)]
pub struct ServerBootstrapReport {
    pub runtime:
        ServerRuntime<BuiltinDispatcher<ApplicationServerInfo<OwnedApplicationServerInfo>>>,
    pub deferred_protocols: Vec<RpcServerPortDeferredProtocol>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerBootstrapConfig {
    pub bind: SocketAddr,
    pub protocols: Vec<String>,
    pub ssl_key: String,
    pub ssl_cert: String,
    pub ssl_chain: String,
    pub elb_support: bool,
    pub start_valid: bool,
    pub skip_ssl_check: bool,
}

impl Default for ServerBootstrapConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            protocols: vec!["http".to_owned(), "ws".to_owned()],
            ssl_key: String::new(),
            ssl_cert: String::new(),
            ssl_chain: String::new(),
            elb_support: false,
            start_valid: false,
            skip_ssl_check: false,
        }
    }
}

pub fn parse_server_bootstrap_args<I>(args: I) -> Result<ServerBootstrapConfig, String>
where
    I: IntoIterator<Item = String>,
{
    let mut config = ServerBootstrapConfig::default();
    let mut iter = args.into_iter();
    let _ = iter.next();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--bind" => {
                let Some(raw_bind) = iter.next() else {
                    return Err("--bind requires a socket address".to_owned());
                };
                config.bind = raw_bind
                    .parse::<SocketAddr>()
                    .map_err(|_| format!("invalid socket address: {raw_bind}"))?;
            }
            "--protocols" => {
                let Some(raw_protocols) = iter.next() else {
                    return Err("--protocols requires a comma-separated list".to_owned());
                };
                config.protocols = parse_protocols(&raw_protocols)?;
            }
            "--ssl-key" => {
                let Some(raw_key) = iter.next() else {
                    return Err("--ssl-key requires a file path".to_owned());
                };
                config.ssl_key = raw_key;
            }
            "--ssl-cert" => {
                let Some(raw_cert) = iter.next() else {
                    return Err("--ssl-cert requires a file path".to_owned());
                };
                config.ssl_cert = raw_cert;
            }
            "--ssl-chain" => {
                let Some(raw_chain) = iter.next() else {
                    return Err("--ssl-chain requires a file path".to_owned());
                };
                config.ssl_chain = raw_chain;
            }
            "--elb-support" => {
                config.elb_support = true;
            }
            "--start-valid" => {
                config.start_valid = true;
            }
            "--help" | "-h" => {
                return Err(usage());
            }
            other => {
                return Err(format!("unrecognized argument: {other}"));
            }
        }
    }

    validate_protocols(&config.protocols)?;
    Ok(config)
}

pub fn build_application_root(config: &ServerBootstrapConfig) -> Result<ApplicationRoot, String> {
    let mut app = ApplicationRoot::with_options(ApplicationRootOptions {
        start_valid: config.start_valid,
        elb_support: config.elb_support,
        ..ApplicationRootOptions::default()
    })
    .map_err(|error| error.to_string())?;
    app.attach_server_ports_setup(build_server_ports_setup_from_bootstrap(config)?);
    Ok(app)
}

pub fn build_runtime(
    config: &ServerBootstrapConfig,
) -> Result<
    ServerRuntime<BuiltinDispatcher<ApplicationServerInfo<OwnedApplicationServerInfo>>>,
    String,
> {
    Ok(build_runtime_report(config)?.runtime)
}

pub fn build_runtime_report(
    config: &ServerBootstrapConfig,
) -> Result<ServerBootstrapReport, String> {
    let app = build_application_root(config)?;
    let report = ServerRuntime::from_application_root_with_report(&app);
    release_bootstrap_app(app)?;
    report.map(|report| ServerBootstrapReport {
        runtime: report.runtime,
        deferred_protocols: report.deferred_protocols,
    })
}

pub fn build_runtime_from_args<I>(
    args: I,
) -> Result<
    ServerRuntime<BuiltinDispatcher<ApplicationServerInfo<OwnedApplicationServerInfo>>>,
    String,
>
where
    I: IntoIterator<Item = String>,
{
    let config = parse_server_bootstrap_args(args)?;
    build_runtime(&config)
}

pub fn build_runtime_report_from_args<I>(args: I) -> Result<ServerBootstrapReport, String>
where
    I: IntoIterator<Item = String>,
{
    let config = parse_server_bootstrap_args(args)?;
    build_runtime_report(&config)
}

fn build_server_ports_setup_from_bootstrap(
    config: &ServerBootstrapConfig,
) -> Result<Arc<ServerPortsSetup>, String> {
    let protocols = config.protocols.clone();
    validate_protocols(&protocols)?;

    let port = ServerPortSetup {
        name: "port_rpc".to_owned(),
        ip: config.bind.ip().to_string(),
        port: config.bind.port(),
        limit: 0,
        protocols,
        user: String::new(),
        password: String::new(),
        admin_user: String::new(),
        admin_password: String::new(),
        admin_nets_v4: Vec::new(),
        admin_nets_v6: Vec::new(),
        ssl_key: config.ssl_key.clone(),
        ssl_cert: config.ssl_cert.clone(),
        ssl_chain: config.ssl_chain.clone(),
        ssl_ciphers: String::new(),
        secure_gateway_nets_v4: Vec::new(),
        secure_gateway_nets_v6: Vec::new(),
    };

    Ok(Arc::new(ServerPortsSetup {
        ports: vec![port],
        client: None,
        overlay: None,
        grpc: None,
    }))
}

fn parse_protocols(raw_protocols: &str) -> Result<Vec<String>, String> {
    let mut protocols = Vec::new();
    for raw in raw_protocols.split(',') {
        let protocol = raw.trim().to_ascii_lowercase();
        if protocol.is_empty() {
            continue;
        }
        if !protocols.iter().any(|candidate| candidate == &protocol) {
            protocols.push(protocol);
        }
    }

    validate_protocols(&protocols)?;
    Ok(protocols)
}

fn validate_protocols(protocols: &[String]) -> Result<(), String> {
    if protocols.is_empty() {
        return Err("at least one transport protocol must be configured".to_owned());
    }

    for protocol in protocols {
        match protocol.as_str() {
            "http" | "ws" | "ws2" => {}
            "peer" | "https" | "wss" | "wss2" => {}
            other => {
                return Err(format!("unsupported bootstrap transport protocol: {other}"));
            }
        }
    }

    Ok(())
}

fn usage() -> String {
    [
        "usage: xrpld-server [--bind ADDR] [--protocols CSV] [--elb-support] [--start-valid]",
        "  --bind ADDR       listen address, default 127.0.0.1:0",
        "  --protocols CSV   comma-separated transport list, default http,ws",
        "  --elb-support     mark the bootstrap app as ELB-capable",
        "  --start-valid     start with the app in a valid network state",
    ]
    .join("\n")
}

fn release_bootstrap_app(app: ApplicationRoot) -> Result<(), String> {
    // `ApplicationRoot` owns a Tokio runtime through `BasicApp`. Dropping that
    // runtime from inside an async caller panics, so the temporary bootstrap
    // root is always released on a plain thread before we return the server
    // runtime shell.
    thread::Builder::new()
        .name("xrpld-bootstrap-app-drop".to_owned())
        .spawn(move || drop(app))
        .map_err(|error| format!("failed to release bootstrap app shell: {error}"))?
        .join()
        .map_err(|_| "bootstrap app shell panicked while shutting down".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        ServerBootstrapConfig, build_runtime, build_runtime_report, parse_server_bootstrap_args,
    };
    use app::ManagedComponent;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn install_crypto() {
        use std::sync::Once;
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        });
    }
    #[test]
    fn parse_server_bootstrap_args_defaults_like_the_simple_server_shell() {
        let config = parse_server_bootstrap_args(["xrpld-server".to_owned()])
            .expect("defaults should parse");
        assert_eq!(
            config.bind,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
        );
        assert_eq!(config.protocols, vec!["http".to_owned(), "ws".to_owned()]);
        assert!(!config.elb_support);
        assert!(!config.start_valid);
    }

    #[test]
    fn parse_server_bootstrap_args_accepts_deferred_peer_and_secure_modes() {
        let config = parse_server_bootstrap_args([
            "xrpld-server".to_owned(),
            "--protocols".to_owned(),
            "http,peer,https".to_owned(),
        ])
        .expect("mixed transport bootstrap should parse");

        assert_eq!(
            config.protocols,
            vec!["http".to_owned(), "peer".to_owned(), "https".to_owned()]
        );
    }

    #[test]
    fn build_runtime_is_safe_to_call_inside_async_contexts() {
        let config = ServerBootstrapConfig {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            protocols: vec!["http".to_owned()],
            ssl_key: String::new(),
            ssl_cert: String::new(),
            ssl_chain: String::new(),
            elb_support: false,
            start_valid: false,
            skip_ssl_check: false,
        };

        let runtime = build_runtime(&config).expect("bootstrap runtime should build");
        runtime.start().expect("runtime should start");
        assert!(
            !runtime.bound_listener_addrs().is_empty(),
            "runtime should bind at least one listener"
        );
        runtime.stop();
    }

    #[test]
    fn build_runtime_report_records_deferred_peer_and_secure_protocols() {
        install_crypto();
        let config = ServerBootstrapConfig {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            protocols: vec!["http".to_owned(), "peer".to_owned(), "https".to_owned()],
            ssl_key: String::new(),
            ssl_cert: String::new(),
            ssl_chain: String::new(),
            elb_support: false,
            start_valid: false,
            skip_ssl_check: false,
        };

        let report = build_runtime_report(&config).expect("bootstrap report should build");
        assert_eq!(report.deferred_protocols.len(), 1);
        assert!(
            report
                .deferred_protocols
                .iter()
                .any(|protocol| protocol.protocol == "peer")
        );
        report.runtime.start().expect("runtime should start");
        assert_eq!(report.runtime.bound_listener_addrs().len(), 1);
        report.runtime.stop();
    }
}
