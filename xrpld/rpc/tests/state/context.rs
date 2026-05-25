//! Tests for context.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use basics::base_uint::Uint256;
use protocol::JsonValue;
use rpc::{
    FeeSource, JsonContextHeaders, LedgerClosed, LedgerClosedSource, LedgerCurrentSource,
    RpcAccessConfig, RpcCommandContext, RpcErrorCode, RpcLoadType, RpcRequestContext, RpcRole,
    ServerInfoSource, SubscriptionManager, do_command,
};

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[derive(Debug)]
struct FakeSource;

impl FeeSource for FakeSource {
    fn fee_json(&self) -> JsonValue {
        JsonValue::Null
    }
}

impl LedgerClosedSource for FakeSource {
    fn closed_ledger(&self) -> Option<LedgerClosed> {
        Some(LedgerClosed {
            seq: 42,
            hash: Uint256::from_u64(0xABCD),
        })
    }
}

impl LedgerCurrentSource for FakeSource {
    fn current_ledger_index(&self) -> u32 {
        321
    }
}

impl ServerInfoSource for FakeSource {
    fn get_server_info(&self, _human: bool, _admin: bool, _counters: bool) -> JsonValue {
        object([("ok", JsonValue::Bool(true))])
    }
}

fn access() -> RpcAccessConfig {
    RpcAccessConfig::default()
}

#[test]
fn request_context_builds_command_context_from_transport_and_session_state() {
    let params = object([]);
    let source = FakeSource;
    let runtime = ();
    let request_headers = BTreeMap::from([
        ("forwarded".to_owned(), "for=203.0.113.9".to_owned()),
        ("x-user".to_owned(), "alice".to_owned()),
    ]);
    let request = RpcRequestContext {
        params: &params,
        env: &source,
        runtime: &runtime,
        role: RpcRole::Identified,
        api_version: 2,
        headers: JsonContextHeaders {
            user: "alice",
            forwarded_for: "203.0.113.9",
        },
        request_headers: request_headers.clone(),
        unlimited: true,
        remote_ip: Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9))),
        load_type: RpcLoadType::Reference,
    };
    let mut session = rpc::InfoSub::with_identity(RpcRole::Identified, "alice", "203.0.113.9");
    let subscriptions = SubscriptionManager::new();
    let access = access();

    let mut context =
        RpcCommandContext::from_request(&request, "ping", &mut session, &subscriptions, &access)
            .expect("request should convert into a command context");

    assert_eq!(context.method, "ping");
    assert_eq!(context.role, RpcRole::Identified);
    assert!(context.unlimited);
    assert_eq!(context.headers, request.headers);
    assert_eq!(context.remote_ip, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)));
    assert_eq!(
        do_command(&mut context),
        object([
            ("ip", JsonValue::String("203.0.113.9".to_owned())),
            ("role", JsonValue::String("identified".to_owned())),
            ("unlimited", JsonValue::Bool(true)),
            ("username", JsonValue::String("alice".to_owned())),
        ])
    );
}

#[test]
fn request_context_shapes_websocket_session_identity() {
    let params = object([]);
    let source = FakeSource;
    let runtime = ();
    let request = RpcRequestContext {
        params: &params,
        env: &source,
        runtime: &runtime,
        role: RpcRole::Identified,
        api_version: 2,
        headers: JsonContextHeaders {
            user: "alice",
            forwarded_for: "203.0.113.9",
        },
        request_headers: BTreeMap::from([
            ("forwarded".to_owned(), "for=203.0.113.9".to_owned()),
            ("x-user".to_owned(), "alice".to_owned()),
        ]),
        unlimited: true,
        remote_ip: Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9))),
        load_type: RpcLoadType::Reference,
    };

    let session =
        request.websocket_session(SocketAddr::from((Ipv4Addr::new(203, 0, 113, 9), 51234)));

    assert_eq!(session.info().role(), RpcRole::Identified);
    assert_eq!(session.api_version(), 2);
    assert_eq!(session.user(), "alice");
    assert_eq!(session.forwarded_for(), "203.0.113.9");
    assert_eq!(session.request_headers(), &request.request_headers);
    assert_eq!(
        session.remote_endpoint(),
        SocketAddr::from((Ipv4Addr::new(203, 0, 113, 9), 51234))
    );
}

#[test]
fn request_context_rejects_missing_remote_ip_like_runtime_should() {
    let params = object([]);
    let source = FakeSource;
    let runtime = ();
    let request = RpcRequestContext {
        params: &params,
        env: &source,
        runtime: &runtime,
        role: RpcRole::Guest,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        request_headers: BTreeMap::new(),
        unlimited: false,
        remote_ip: None,
        load_type: RpcLoadType::Reference,
    };
    let mut session = rpc::InfoSub::new(RpcRole::Guest);
    let subscriptions = SubscriptionManager::new();
    let access = access();

    let error = RpcCommandContext::from_request(
        &request,
        "server_info",
        &mut session,
        &subscriptions,
        &access,
    )
    .expect_err("missing remote ip should fail");

    assert_eq!(error.error_code(), Some(RpcErrorCode::Internal));
}
