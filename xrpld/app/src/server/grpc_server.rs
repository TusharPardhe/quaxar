//! In-memory gRPC server binding surface aligned with `GRPCServer.h`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use protocol::JsonValue;

use crate::runtime::main_runtime::ManagedComponent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrpcStatusCode {
    Ok,
    InvalidArgument,
    PermissionDenied,
    FailedPrecondition,
    ResourceExhausted,
    Unavailable,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrpcStatus {
    pub code: GrpcStatusCode,
    pub message: String,
}

impl GrpcStatus {
    pub fn ok() -> Self {
        Self {
            code: GrpcStatusCode::Ok,
            message: String::new(),
        }
    }

    pub fn new(code: GrpcStatusCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn is_ok(&self) -> bool {
        self.code == GrpcStatusCode::Ok
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrpcCondition {
    None,
    Network,
    ValidatedLedger,
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrpcLoadType {
    Unlimited,
    MediumBurden,
    Burden,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GrpcClient {
    pub ip: Option<String>,
    pub proxied_ip: Option<String>,
    pub user: Option<String>,
    pub is_unlimited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrpcCallContext {
    pub method: String,
    pub request: JsonValue,
    pub client: GrpcClient,
    pub api_version: u32,
    pub required_condition: GrpcCondition,
    pub load_type: GrpcLoadType,
    pub secure_gateway_ips: Vec<String>,
}

impl GrpcCallContext {
    pub fn client_ip(&self) -> Option<&str> {
        self.client.ip.as_deref()
    }

    pub fn proxied_client_ip(&self) -> Option<&str> {
        self.client.proxied_ip.as_deref()
    }

    pub fn is_secure_gateway(&self) -> bool {
        self.client
            .ip
            .as_ref()
            .is_some_and(|ip| self.secure_gateway_ips.iter().any(|allowed| allowed == ip))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrpcCallOutput {
    pub response: JsonValue,
    pub status: GrpcStatus,
    pub is_unlimited: bool,
    pub forwarded: bool,
}

impl GrpcCallOutput {
    pub fn ok(response: JsonValue, is_unlimited: bool) -> Self {
        Self {
            response,
            status: GrpcStatus::ok(),
            is_unlimited,
            forwarded: false,
        }
    }
}

pub trait GrpcMethodHandler: Send + Sync + 'static {
    fn handle(&self, context: &GrpcCallContext) -> GrpcCallOutput;
}

impl<F> GrpcMethodHandler for F
where
    F: Fn(&GrpcCallContext) -> GrpcCallOutput + Send + Sync + 'static,
{
    fn handle(&self, context: &GrpcCallContext) -> GrpcCallOutput {
        self(context)
    }
}

pub trait GrpcForwarder: Send + Sync + 'static {
    fn forward(&self, context: &GrpcCallContext) -> Option<GrpcCallOutput>;
}

impl<F> GrpcForwarder for F
where
    F: Fn(&GrpcCallContext) -> Option<GrpcCallOutput> + Send + Sync + 'static,
{
    fn forward(&self, context: &GrpcCallContext) -> Option<GrpcCallOutput> {
        self(context)
    }
}

#[derive(Clone)]
pub struct GrpcMethodBinding {
    pub method: String,
    pub handler: Arc<dyn GrpcMethodHandler>,
    pub forwarder: Option<Arc<dyn GrpcForwarder>>,
    pub required_condition: GrpcCondition,
    pub load_type: GrpcLoadType,
}

impl std::fmt::Debug for GrpcMethodBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrpcMethodBinding")
            .field("method", &self.method)
            .field("has_forwarder", &self.forwarder.is_some())
            .field("required_condition", &self.required_condition)
            .field("load_type", &self.load_type)
            .finish()
    }
}

pub trait Processor: Send + Sync {
    fn process(&self);
    fn clone_processor(&self) -> Arc<dyn Processor>;
    fn is_finished(&self) -> bool;
}

#[derive(Clone)]
struct GrpcCallData {
    binding: GrpcMethodBinding,
    secure_gateway_ips: Vec<String>,
    request: Arc<Mutex<Option<GrpcCallContext>>>,
    output: Arc<Mutex<Option<GrpcCallOutput>>>,
    finished: Arc<AtomicBool>,
}

impl GrpcCallData {
    fn new(binding: GrpcMethodBinding, secure_gateway_ips: Vec<String>) -> Self {
        Self {
            binding,
            secure_gateway_ips,
            request: Arc::new(Mutex::new(None)),
            output: Arc::new(Mutex::new(None)),
            finished: Arc::new(AtomicBool::new(false)),
        }
    }

    fn with_request(&self, request: JsonValue, client: GrpcClient, api_version: u32) -> Self {
        let next = Self::new(self.binding.clone(), self.secure_gateway_ips.clone());
        *next.request.lock().expect("request mutex") = Some(GrpcCallContext {
            method: self.binding.method.clone(),
            request,
            client,
            api_version,
            required_condition: self.binding.required_condition,
            load_type: self.binding.load_type,
            secure_gateway_ips: self.secure_gateway_ips.clone(),
        });
        next
    }

    fn take_output(&self) -> Option<GrpcCallOutput> {
        self.output.lock().expect("output mutex").clone()
    }
}

impl Processor for GrpcCallData {
    fn process(&self) {
        let request = self.request.lock().expect("request mutex").take();
        let Some(context) = request else {
            self.finished.store(true, Ordering::Release);
            return;
        };

        let mut output = self
            .binding
            .forwarder
            .as_ref()
            .and_then(|forwarder| forwarder.forward(&context))
            .unwrap_or_else(|| self.binding.handler.handle(&context));

        if output.status.is_ok() {
            output.is_unlimited = context.client.is_unlimited;
        }

        *self.output.lock().expect("output mutex") = Some(output);
        self.finished.store(true, Ordering::Release);
    }

    fn clone_processor(&self) -> Arc<dyn Processor> {
        Arc::new(Self::new(
            self.binding.clone(),
            self.secure_gateway_ips.clone(),
        ))
    }

    fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}

pub struct GrpcServerImpl {
    server_address: String,
    server_port: u16,
    secure_gateway_ips: Vec<String>,
    listeners: Mutex<BTreeMap<String, Arc<GrpcCallData>>>,
    started: AtomicBool,
}

impl std::fmt::Debug for GrpcServerImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrpcServerImpl")
            .field("server_address", &self.server_address)
            .field("server_port", &self.server_port)
            .field("secure_gateway_ips", &self.secure_gateway_ips)
            .field(
                "listener_count",
                &self.listeners.lock().expect("listeners").len(),
            )
            .field("started", &self.started.load(Ordering::Acquire))
            .finish()
    }
}

impl GrpcServerImpl {
    pub fn new(
        server_address: impl Into<String>,
        server_port: u16,
        secure_gateway_ips: Vec<String>,
    ) -> Self {
        Self {
            server_address: server_address.into(),
            server_port,
            secure_gateway_ips,
            listeners: Mutex::new(BTreeMap::new()),
            started: AtomicBool::new(false),
        }
    }

    pub fn register_method(&self, binding: GrpcMethodBinding) {
        let method = binding.method.clone();
        self.listeners.lock().expect("listeners mutex").insert(
            method,
            Arc::new(GrpcCallData::new(binding, self.secure_gateway_ips.clone())),
        );
    }

    pub fn shutdown(&self) {
        self.started.store(false, Ordering::Release);
    }

    pub fn start_server(&self) -> bool {
        self.started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub fn handle_rpcs(&self) {}

    pub fn setup_listeners(&self) -> Vec<Arc<dyn Processor>> {
        self.listeners
            .lock()
            .expect("listeners mutex")
            .values()
            .cloned()
            .map(|listener| listener as Arc<dyn Processor>)
            .collect()
    }

    pub fn get_endpoint(&self) -> String {
        format!("{}:{}", self.server_address, self.server_port)
    }

    pub fn dispatch(
        &self,
        method: &str,
        request: JsonValue,
        client: GrpcClient,
        api_version: u32,
    ) -> Result<GrpcCallOutput, GrpcStatus> {
        if !self.started.load(Ordering::Acquire) {
            return Err(GrpcStatus::new(
                GrpcStatusCode::Unavailable,
                "gRPC server is not running",
            ));
        }

        let listener = self
            .listeners
            .lock()
            .expect("listeners mutex")
            .get(method)
            .cloned()
            .ok_or_else(|| {
                GrpcStatus::new(
                    GrpcStatusCode::InvalidArgument,
                    format!("unknown gRPC method: {method}"),
                )
            })?;

        let call = listener.with_request(request, client, api_version);
        call.process();
        call.take_output()
            .ok_or_else(|| GrpcStatus::new(GrpcStatusCode::Internal, "missing gRPC response"))
    }
}

impl ManagedComponent for GrpcServerImpl {
    fn start(&self) -> Result<(), String> {
        if self.start_server() {
            Ok(())
        } else {
            Err("gRPC server already started".to_owned())
        }
    }

    fn stop(&self) {
        self.shutdown();
    }

    fn fd_required(&self) -> usize {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GrpcCallOutput, GrpcClient, GrpcCondition, GrpcLoadType, GrpcMethodBinding, GrpcServerImpl,
        GrpcStatus, GrpcStatusCode,
    };
    use crate::runtime::main_runtime::ManagedComponent;
    use protocol::JsonValue;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
        JsonValue::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    #[test]
    fn grpc_server_dispatches_local_handler_and_reports_unlimited() {
        let server = GrpcServerImpl::new("127.0.0.1", 50051, vec!["127.0.0.1".to_owned()]);
        server.register_method(GrpcMethodBinding {
            method: "SubmitTransaction".to_owned(),
            handler: Arc::new(|context: &super::GrpcCallContext| {
                assert!(context.is_secure_gateway());
                assert_eq!(context.client_ip(), Some("127.0.0.1"));
                GrpcCallOutput::ok(
                    object([("method", JsonValue::String(context.method.clone()))]),
                    false,
                )
            }),
            forwarder: None,
            required_condition: GrpcCondition::Network,
            load_type: GrpcLoadType::MediumBurden,
        });

        assert!(server.start_server());
        let output = server
            .dispatch(
                "SubmitTransaction",
                JsonValue::Null,
                GrpcClient {
                    ip: Some("127.0.0.1".to_owned()),
                    proxied_ip: None,
                    user: Some("alice".to_owned()),
                    is_unlimited: true,
                },
                1,
            )
            .expect("dispatch should succeed");

        assert!(output.status.is_ok());
        assert!(output.is_unlimited);
        assert!(!output.forwarded);
    }

    #[test]
    fn grpc_server_uses_forwarder_before_local_handler() {
        let server = GrpcServerImpl::new("127.0.0.1", 50052, Vec::new());
        let local_calls = Arc::new(AtomicUsize::new(0));
        let local_calls_for_handler = Arc::clone(&local_calls);
        server.register_method(GrpcMethodBinding {
            method: "GetLedger".to_owned(),
            handler: Arc::new(move |_context: &super::GrpcCallContext| {
                local_calls_for_handler.fetch_add(1, Ordering::AcqRel);
                GrpcCallOutput::ok(JsonValue::String("local".to_owned()), false)
            }),
            forwarder: Some(Arc::new(|_context: &super::GrpcCallContext| {
                Some(GrpcCallOutput {
                    response: JsonValue::String("forwarded".to_owned()),
                    status: GrpcStatus::ok(),
                    is_unlimited: false,
                    forwarded: true,
                })
            })),
            required_condition: GrpcCondition::ValidatedLedger,
            load_type: GrpcLoadType::Burden,
        });

        assert!(server.start_server());
        let output = server
            .dispatch("GetLedger", JsonValue::Null, GrpcClient::default(), 1)
            .expect("dispatch should succeed");

        assert_eq!(output.response, JsonValue::String("forwarded".to_owned()));
        assert!(output.forwarded);
        assert_eq!(local_calls.load(Ordering::Acquire), 0);
    }

    #[test]
    fn grpc_server_listener_clone_and_lifecycle_match_header_contracts() {
        let server = GrpcServerImpl::new("0.0.0.0", 60000, Vec::new());
        server.register_method(GrpcMethodBinding {
            method: "Ping".to_owned(),
            handler: Arc::new(|_context: &super::GrpcCallContext| {
                GrpcCallOutput::ok(JsonValue::Bool(true), false)
            }),
            forwarder: None,
            required_condition: GrpcCondition::None,
            load_type: GrpcLoadType::Unlimited,
        });

        let listeners = server.setup_listeners();
        assert_eq!(listeners.len(), 1);
        assert!(!listeners[0].is_finished());

        let cloned = listeners[0].clone_processor();
        assert!(!cloned.is_finished());

        assert_eq!(server.get_endpoint(), "0.0.0.0:60000");
        assert!(server.start().is_ok());
        assert!(server.start().is_err());
        server.stop();
        assert_eq!(server.fd_required(), 1);
    }

    #[test]
    fn grpc_server_rejects_unknown_methods_and_stopped_dispatch() {
        let server = GrpcServerImpl::new("127.0.0.1", 50053, Vec::new());
        let stopped = server
            .dispatch("Unknown", JsonValue::Null, GrpcClient::default(), 1)
            .expect_err("stopped server should reject dispatch");
        assert_eq!(stopped.code, GrpcStatusCode::Unavailable);

        assert!(server.start_server());
        let unknown = server
            .dispatch("Unknown", JsonValue::Null, GrpcClient::default(), 1)
            .expect_err("unknown method should fail");
        assert_eq!(unknown.code, GrpcStatusCode::InvalidArgument);
    }
}
