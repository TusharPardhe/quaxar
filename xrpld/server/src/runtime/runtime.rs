use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use app::{ApplicationRoot, ManagedComponent};
use rpc::{ApplicationServerInfo, OwnedApplicationServerInfo};
use tokio::sync::Notify;

use crate::{
    BuiltinDispatcher, OwnedServerStatusSource, RpcDispatcher, RpcServer, RpcServerPortBuild,
    RpcServerPortDeferredProtocol, RpcServerPortPolicy, ServerStatusSource, SubscriptionManager,
};

#[derive(Default)]
struct ServerRuntimeState {
    started: AtomicBool,
    shutdown: AtomicBool,
    active_servers: Mutex<usize>,
    bound_addresses: Mutex<Vec<SocketAddr>>,
    listener_threads: Mutex<Vec<JoinHandle<()>>>,
    shutdown_notify: Notify,
    stopped: Condvar,
}

impl ServerRuntimeState {
    fn clear_started(&self) {
        self.started.store(false, Ordering::Release);
    }

    fn mark_started(&self) -> Result<(), String> {
        self.started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| "server runtime already started".to_owned())
    }

    fn register_servers(&self, count: usize) {
        let mut active = self
            .active_servers
            .lock()
            .expect("server runtime active count mutex poisoned");
        *active = count;
    }

    fn set_bound_addresses(&self, addresses: Vec<SocketAddr>) {
        let mut guard = self
            .bound_addresses
            .lock()
            .expect("server runtime addresses mutex poisoned");
        *guard = addresses;
    }

    fn clear_bound_addresses(&self) {
        self.bound_addresses
            .lock()
            .expect("server runtime addresses mutex poisoned")
            .clear();
    }

    fn push_listener_thread(&self, handle: JoinHandle<()>) {
        self.listener_threads
            .lock()
            .expect("server runtime listener threads mutex poisoned")
            .push(handle);
    }

    fn take_listener_threads(&self) -> Vec<JoinHandle<()>> {
        self.listener_threads
            .lock()
            .expect("server runtime listener threads mutex poisoned")
            .drain(..)
            .collect()
    }

    fn notify_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.shutdown_notify.notify_waiters();
    }

    fn clear_shutdown(&self) {
        self.shutdown.store(false, Ordering::Release);
    }

    async fn wait_for_shutdown(self: Arc<Self>) {
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                break;
            }
            self.shutdown_notify.notified().await;
        }
    }

    fn server_finished(&self) {
        let mut active = self
            .active_servers
            .lock()
            .expect("server runtime active count mutex poisoned");
        if *active > 0 {
            *active -= 1;
        }
        if *active == 0 {
            self.stopped.notify_all();
        }
    }

    fn wait_for_stop(&self) {
        let mut active = self
            .active_servers
            .lock()
            .expect("server runtime active count mutex poisoned");
        while *active > 0 {
            active = self
                .stopped
                .wait(active)
                .expect("server runtime stop condvar poisoned");
        }
    }

    fn bound_addresses(&self) -> Vec<SocketAddr> {
        self.bound_addresses
            .lock()
            .expect("server runtime addresses mutex poisoned")
            .clone()
    }

    fn active_listener_count(&self) -> usize {
        *self
            .active_servers
            .lock()
            .expect("server runtime active count mutex poisoned")
    }
}

impl std::fmt::Debug for ServerRuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let active_servers = *self
            .active_servers
            .lock()
            .expect("server runtime active count mutex poisoned");
        let bound_addresses = self.bound_addresses();
        let listener_threads = self
            .listener_threads
            .lock()
            .expect("server runtime listener threads mutex poisoned")
            .len();
        f.debug_struct("ServerRuntimeState")
            .field("started", &self.started.load(Ordering::Acquire))
            .field("shutdown", &self.shutdown.load(Ordering::Acquire))
            .field("active_servers", &active_servers)
            .field("bound_addresses", &bound_addresses)
            .field("listener_threads", &listener_threads)
            .finish()
    }
}

#[derive(Clone)]
pub struct ServerRuntime<D> {
    dispatcher: D,
    policies: Vec<RpcServerPortPolicy>,
    status_source: Option<Arc<dyn ServerStatusSource>>,
    state: Arc<ServerRuntimeState>,
    deferred_protocols: Vec<RpcServerPortDeferredProtocol>,
    /// Shared subscription manager across ALL ports. Ensures that a
    /// publish from the HTTP handler reaches WS subscribers.
    shared_subscriptions: Arc<crate::SubscriptionManager>,
}

pub struct ServerRuntimeBuildReport<D> {
    pub runtime: ServerRuntime<D>,
    pub deferred_protocols: Vec<RpcServerPortDeferredProtocol>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerTransportReport {
    pub bound_addresses: Vec<SocketAddr>,
    pub active_listener_count: usize,
    pub deferred_protocols: Vec<RpcServerPortDeferredProtocol>,
}

impl ServerTransportReport {
    pub fn deferred_peer_handoff_count(&self) -> usize {
        self.deferred_protocols
            .iter()
            .filter(|protocol| protocol.is_peer_handoff())
            .count()
    }

    pub fn deferred_secure_listener_count(&self) -> usize {
        self.deferred_protocols
            .iter()
            .filter(|protocol| protocol.is_secure_listener())
            .count()
    }

    pub fn deferred_transport_summary(&self) -> String {
        let peer_handoff = self.deferred_peer_handoff_count();
        let secure_listener = self.deferred_secure_listener_count();

        if peer_handoff == 0 && secure_listener == 0 {
            return "no deferred peer handoff or secure listener transports".to_owned();
        }

        format!(
            "{peer_handoff} peer handoff transport(s) deferred, {secure_listener} secure listener transport(s) deferred"
        )
    }
}

impl<D> std::fmt::Debug for ServerRuntime<D>
where
    D: RpcDispatcher + Clone + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerRuntime")
            .field("policy_count", &self.policies.len())
            .field("started", &self.state.started.load(Ordering::Acquire))
            .field("shutdown", &self.state.shutdown.load(Ordering::Acquire))
            .field("bound_addresses", &self.bound_listener_addrs())
            .field("deferred_protocols", &self.deferred_protocols)
            .finish()
    }
}

impl<D> ServerRuntime<D>
where
    D: RpcDispatcher + Clone + Send + Sync + 'static,
{
    pub fn new(
        _handle: tokio::runtime::Handle,
        dispatcher: D,
        policies: Vec<RpcServerPortPolicy>,
    ) -> Self {
        Self::with_status_source(_handle, dispatcher, policies, None)
    }

    pub fn with_status_source(
        _handle: tokio::runtime::Handle,
        dispatcher: D,
        policies: Vec<RpcServerPortPolicy>,
        status_source: Option<Arc<dyn ServerStatusSource>>,
    ) -> Self {
        Self {
            dispatcher,
            policies,
            status_source,
            state: Arc::new(ServerRuntimeState::default()),
            deferred_protocols: Vec::new(),
            shared_subscriptions: Arc::new(crate::SubscriptionManager::default()),
        }
    }

    pub fn bound_listener_addrs(&self) -> Vec<SocketAddr> {
        self.state.bound_addresses()
    }

    pub fn transport_report(&self) -> ServerTransportReport {
        ServerTransportReport {
            bound_addresses: self.bound_listener_addrs(),
            active_listener_count: self.state.active_listener_count(),
            deferred_protocols: self.deferred_protocols.clone(),
        }
    }

    fn bind_listener(
        policy: &RpcServerPortPolicy,
    ) -> std::io::Result<(StdTcpListener, SocketAddr)> {
        let listener = StdTcpListener::bind(policy.socket_addr)?;
        let local_addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        Ok((listener, local_addr))
    }

    fn spawn_listener(
        &self,
        policy: RpcServerPortPolicy,
        listener: StdTcpListener,
    ) -> Result<(), String> {
        let listener_name = policy.name.clone();
        let server = RpcServer::with_auth_and_subscriptions(
            self.dispatcher.clone(),
            crate::auth::ServerAuth::new(policy.auth.clone()),
            Arc::clone(&self.shared_subscriptions),
        );
        let state = Arc::clone(&self.state);
        let shutdown = Arc::clone(&self.state);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread_name = format!("xrpld-server-{}", listener_name);
        let thread = thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                struct ListenerGuard(Arc<ServerRuntimeState>);

                impl Drop for ListenerGuard {
                    fn drop(&mut self) {
                        self.0.server_finished();
                    }
                }

                let _guard = ListenerGuard(Arc::clone(&state));
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(4)
                    .max_blocking_threads(128)
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_tx.send(Err(format!(
                            "server runtime failed to build listener runtime: {error}"
                        )));
                        return;
                    }
                };

                let rustls_config = policy
                    .tls_config
                    .map(axum_server::tls_rustls::RustlsConfig::from_config);

                let _ = ready_tx.send(Ok(()));
                runtime.block_on(async move {
                    let addr = listener.local_addr().expect("failed to get local addr");

                    let router = server
                        .router()
                        .into_make_service_with_connect_info::<SocketAddr>();

                    if let Some(config) = rustls_config {
                        let handle = axum_server::Handle::new();
                        let shutdown_handle = handle.clone();
                        let shutdown_state = Arc::clone(&shutdown);
                        tokio::spawn(async move {
                            shutdown_state.wait_for_shutdown().await;
                            shutdown_handle.shutdown();
                        });
                        match axum_server::from_tcp_rustls(listener, config) {
                            Ok(server) => {
                                if let Err(error) = server.handle(handle).serve(router).await {
                                    tracing::warn!(target: "server",
                                        "server runtime secure listener stopped with error: {error}"
                                    );
                                }
                            }
                            Err(error) => {
                                tracing::warn!(target: "server",
                                    "server runtime secure listener adoption failed on {addr}: {error}"
                                );
                            }
                        }
                    } else {
                        let bound_listener = tokio::net::TcpListener::from_std(listener)
                            .expect("failed to adopt std listener");
                        if let Err(error) = axum::serve(bound_listener, router)
                            .with_graceful_shutdown(shutdown.wait_for_shutdown())
                            .await
                        {
                            tracing::warn!(target: "server", "server runtime plain listener stopped with error: {error}");
                        }
                    }
                });
            })
            .map_err(|error| error.to_string())?;
        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.state.push_listener_thread(thread);
                Ok(())
            }
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(error) => {
                let _ = thread.join();
                Err(format!(
                    "server runtime listener thread failed to start: {error}"
                ))
            }
        }
    }

    fn join_listener_threads(&self) {
        for thread in self.state.take_listener_threads() {
            if let Err(error) = thread.join() {
                tracing::warn!(target: "server", "server runtime listener thread panicked: {:?}", error);
            }
        }
    }

    fn rollback_start(&self, started_servers: usize) {
        self.state.register_servers(started_servers);
        self.state.notify_shutdown();
        self.state.wait_for_stop();
        self.join_listener_threads();
        self.state.clear_bound_addresses();
        self.state.clear_shutdown();
        self.state.clear_started();
    }
}

impl ServerRuntime<BuiltinDispatcher<ApplicationServerInfo<OwnedApplicationServerInfo>>> {
    pub fn from_application_root_with_report(
        app: &ApplicationRoot,
    ) -> Result<
        ServerRuntimeBuildReport<
            BuiltinDispatcher<ApplicationServerInfo<OwnedApplicationServerInfo>>,
        >,
        String,
    > {
        let setup = app
            .server_ports_setup()
            .ok_or_else(|| "application root does not have a server ports setup".to_owned())?;

        let mut policies = Vec::with_capacity(setup.ports.len());
        let mut deferred_protocols = Vec::new();
        for port in &setup.ports {
            let build = RpcServerPortBuild::from_server_port(port)?;
            if let Some(policy) = build.policy {
                policies.push(policy);
            }
            deferred_protocols.extend(build.deferred_protocols);
        }

        if policies.is_empty() {
            let deferred = if deferred_protocols.is_empty() {
                String::new()
            } else {
                let details = deferred_protocols
                    .iter()
                    .map(|protocol| format!("{} on {}", protocol.protocol, protocol.port_name))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("; deferred {details}")
            };
            return Err(format!(
                "server ports setup does not expose any supported Rust HTTP/WS listeners{deferred}"
            ));
        }

        let shared_subs = Arc::new(SubscriptionManager::default());
        let source =
            ApplicationServerInfo::new(OwnedApplicationServerInfo::from_application_root(app));
        let path_source: Arc<dyn rpc::PathFinderSource + Send + Sync> = Arc::new(source.clone());
        let dispatcher = BuiltinDispatcher::new(source, (*shared_subs).clone())
            .with_path_find(Arc::new(rpc::PathRequestManager::new()), path_source);
        let mut runtime = Self::with_status_source(
            app.basic_app().handle(),
            dispatcher,
            policies,
            Some(Arc::new(OwnedServerStatusSource::from_application_root(
                app,
            ))),
        );
        runtime.shared_subscriptions = shared_subs;
        runtime.deferred_protocols = deferred_protocols.clone();
        Ok(ServerRuntimeBuildReport {
            runtime,
            deferred_protocols,
        })
    }

    pub fn from_application_root(app: &ApplicationRoot) -> Result<Self, String> {
        Ok(Self::from_application_root_with_report(app)?.runtime)
    }

    pub fn path_find_tuning(&self) -> Option<rpc::PathFindTuning> {
        self.dispatcher
            .path_source
            .as_ref()
            .map(|source| source.path_find_tuning())
    }
}

impl<S> ServerRuntime<crate::BuiltinDispatcher<S>>
where
    S: Clone + Send + Sync + 'static,
{
    pub fn subscriptions(&self) -> crate::SubscriptionManager {
        (*self.shared_subscriptions).clone()
    }
}

impl<D> ManagedComponent for ServerRuntime<D>
where
    D: RpcDispatcher + Clone + Send + Sync + 'static,
{
    fn start(&self) -> Result<(), String> {
        if self.policies.is_empty() {
            return Err("server runtime needs at least one port policy".to_owned());
        }
        self.state.mark_started()?;

        let mut bound = Vec::with_capacity(self.policies.len());
        for policy in &self.policies {
            match Self::bind_listener(policy) {
                Ok((listener, local_addr)) => {
                    bound.push((policy.clone(), listener, local_addr));
                }
                Err(error) => {
                    self.state.clear_started();
                    return Err(error.to_string());
                }
            }
        }

        self.state
            .set_bound_addresses(bound.iter().map(|(_, _, addr)| *addr).collect());

        let mut started_servers = 0_usize;
        for (policy, listener, _) in bound {
            let port = policy.socket_addr.port();
            let protocol = if policy.tls_config.is_some() {
                "https"
            } else {
                "http"
            };
            if let Err(error) = self.spawn_listener(policy, listener) {
                self.rollback_start(started_servers);
                return Err(error);
            }
            tracing::info!(target: "server", port, protocol, "Server listening");
            started_servers += 1;
            self.state.register_servers(started_servers);
        }

        tracing::info!(target: "server", "Server started successfully");
        Ok(())
    }

    fn stop(&self) {
        tracing::info!(target: "server", "Server shutting down");
        self.state.notify_shutdown();
        self.state.wait_for_stop();
        self.join_listener_threads();
        self.state.clear_bound_addresses();
        self.state.clear_shutdown();
    }

    fn fd_required(&self) -> usize {
        self.policies.len()
    }
}

#[cfg(test)]
mod tests {
    use super::ServerRuntime;
    use crate::{RpcDispatcher, RpcReply, RpcRequest, RpcServerPortPolicy, ServerAuthConfig};
    use app::{
        ApplicationRoot, ManagedComponent, ServerPortClientSetup, ServerPortOverlaySetup,
        ServerPortSetup, ServerPortsSetup,
    };
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::{Arc, Mutex};
    use tokio::runtime::Handle;

    #[derive(Clone, Default)]
    struct RecordingDispatcher {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingDispatcher {
        fn calls(&self) -> Vec<String> {
            self.calls
                .lock()
                .expect("dispatcher calls mutex poisoned")
                .clone()
        }
    }

    impl RpcDispatcher for RecordingDispatcher {
        fn dispatch(&self, request: RpcRequest<'_>) -> RpcReply {
            self.calls
                .lock()
                .expect("dispatcher calls mutex poisoned")
                .push(request.method.to_owned());

            RpcReply::result(protocol::JsonValue::Object(BTreeMap::from([(
                "ok".to_owned(),
                protocol::JsonValue::Bool(true),
            )])))
        }
    }

    fn policy(name: &str, port: u16, allow_http: bool, allow_ws: bool) -> RpcServerPortPolicy {
        RpcServerPortPolicy {
            name: name.to_owned(),
            socket_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            allow_http,
            allow_ws,
            auth: ServerAuthConfig::default(),
            tls_config: None,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn server_runtime_binds_multiple_plain_listeners_and_serves_requests() {
        let dispatcher = RecordingDispatcher::default();
        let runtime = ServerRuntime::new(
            Handle::current(),
            dispatcher.clone(),
            vec![policy("http", 0, true, false), policy("ws", 0, false, true)],
        );

        runtime.start().expect("runtime should start");
        let addrs = runtime.bound_listener_addrs();
        assert_eq!(addrs.len(), 2);
        assert_ne!(addrs[0], addrs[1]);

        let http_response = tokio::task::spawn_blocking({
            let addr = addrs[0];
            move || {
                let mut stream = std::net::TcpStream::connect(addr)
                    .expect("client should connect to http listener");
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .expect("read timeout should set");
                stream
                    .write_all(
                        concat!(
                            "POST / HTTP/1.1\r\n",
                            "Host: localhost\r\n",
                            "Content-Type: application/json\r\n",
                            "Content-Length: 40\r\n",
                            "\r\n",
                            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}"
                        )
                        .as_bytes(),
                    )
                    .expect("client should write request");
                let mut response = Vec::new();
                let mut chunk = [0_u8; 256];
                loop {
                    let read = stream
                        .read(&mut chunk)
                        .expect("client should read response");
                    if read == 0 {
                        break;
                    }
                    response.extend_from_slice(&chunk[..read]);
                    if response.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                String::from_utf8(response).expect("response should be utf-8")
            }
        })
        .await
        .expect("blocking client should finish");
        assert!(http_response.contains("200 OK"));

        let ws_response = tokio::task::spawn_blocking({
            let addr = addrs[1];
            move || {
                let mut stream = std::net::TcpStream::connect(addr)
                    .expect("client should connect to ws listener");
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .expect("read timeout should set");
                stream
                    .write_all(
                        concat!(
                            "GET / HTTP/1.1\r\n",
                            "Host: localhost\r\n",
                            "Connection: upgrade\r\n",
                            "Upgrade: websocket\r\n",
                            "Sec-WebSocket-Version: 13\r\n",
                            "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n",
                            "\r\n",
                        )
                        .as_bytes(),
                    )
                    .expect("client should write request");
                let mut response = Vec::new();
                let mut chunk = [0_u8; 256];
                loop {
                    let read = stream
                        .read(&mut chunk)
                        .expect("client should read response");
                    if read == 0 {
                        break;
                    }
                    response.extend_from_slice(&chunk[..read]);
                    if response.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                String::from_utf8(response).expect("response should be utf-8")
            }
        })
        .await
        .expect("blocking client should finish");
        assert!(ws_response.starts_with("HTTP/1.1 101 Switching Protocols"));

        runtime.stop();
        assert_eq!(dispatcher.calls(), vec!["ping".to_owned()]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn server_runtime_stop_releases_listener_ports_cleanly() {
        let runtime = ServerRuntime::new(
            Handle::current(),
            RecordingDispatcher::default(),
            vec![policy("http", 0, true, false)],
        );

        runtime.start().expect("runtime should start");
        let addr = runtime.bound_listener_addrs()[0];

        runtime.stop();

        let rebound = std::net::TcpListener::bind(addr);
        assert!(
            rebound.is_ok(),
            "listener port should be released after stop"
        );
    }

    #[test]
    fn server_runtime_builds_from_application_root_setup_boundary() {
        let mut app = ApplicationRoot::new(1).expect("root shell should build");
        let setup = Arc::new(ServerPortsSetup {
            ports: vec![ServerPortSetup {
                name: "port_rpc".to_owned(),
                ip: "127.0.0.1".to_owned(),
                port: 0,
                limit: 0,
                protocols: vec!["http".to_owned(), "ws".to_owned()],
                user: String::new(),
                password: String::new(),
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
            }],
            client: Some(ServerPortClientSetup {
                secure: false,
                ip: "127.0.0.1".to_owned(),
                port: 0,
                user: String::new(),
                password: String::new(),
                admin_user: String::new(),
                admin_password: String::new(),
            }),
            overlay: Some(ServerPortOverlaySetup {
                ip: "127.0.0.1".to_owned(),
                port: 0,
                limit: 0,
                secure: false,
                ssl_key: String::new(),
                ssl_cert: String::new(),
                ssl_chain: String::new(),
                ssl_ciphers: String::new(),
            }),
            grpc: None,
        });
        app.attach_server_ports_setup(setup);

        let runtime =
            ServerRuntime::from_application_root(&app).expect("runtime should build from app");

        assert_eq!(runtime.fd_required(), 1);
        assert!(runtime.dispatcher.path_requests.is_some());
        assert!(runtime.dispatcher.path_source.is_some());
        let tuning = runtime
            .dispatcher
            .path_source
            .as_ref()
            .expect("path source should be attached")
            .path_find_tuning();
        assert_eq!(tuning.old, 2);
        assert_eq!(tuning.search, 2);
        assert_eq!(tuning.fast, 2);
        assert_eq!(tuning.max, 3);
    }

    #[test]
    fn server_runtime_builds_from_current_thread_application_root() {
        let mut app = ApplicationRoot::new(0).expect("root shell should build");
        app.attach_server_ports_setup(Arc::new(ServerPortsSetup {
            ports: vec![ServerPortSetup {
                name: "port_rpc".to_owned(),
                ip: "127.0.0.1".to_owned(),
                port: 0,
                limit: 0,
                protocols: vec!["http".to_owned()],
                user: String::new(),
                password: String::new(),
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
            }],
            client: Some(ServerPortClientSetup {
                secure: false,
                ip: "127.0.0.1".to_owned(),
                port: 0,
                user: String::new(),
                password: String::new(),
                admin_user: String::new(),
                admin_password: String::new(),
            }),
            overlay: None,
            grpc: None,
        }));

        let runtime =
            ServerRuntime::from_application_root(&app).expect("current-thread app should build");
        runtime
            .start()
            .expect("current-thread runtime should start");
        assert_eq!(runtime.bound_listener_addrs().len(), 1);
        runtime.stop();
    }
}
