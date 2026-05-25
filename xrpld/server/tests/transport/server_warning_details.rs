use app::{ApplicationRoot, UnsupportedMajorityWarningDetails};
use protocol::JsonValue;
use server::{BuiltinDispatcher, RequestMetadata, RpcDispatcher, RpcRequest, SubscriptionManager};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

fn metadata(role: rpc::RpcRole) -> RequestMetadata {
    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let mut metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );
    metadata.role = role;
    metadata.unlimited = matches!(role, rpc::RpcRole::Admin);
    metadata
}

#[test]
fn dispatcher_surfaces_unsupported_majority_warning_details_for_admin() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_unsupported_majority_warning_details(Some(UnsupportedMajorityWarningDetails {
        expected_date: 1_700_000_000,
        expected_date_utc: "2023-Nov-14 22:13:20 UTC".to_owned(),
    }));
    let dispatcher = BuiltinDispatcher::new(
        rpc::ApplicationServerInfo::new(&app),
        SubscriptionManager::default(),
    );
    let admin = metadata(rpc::RpcRole::Admin);

    let reply = dispatcher.dispatch(RpcRequest {
        method: "server_info",
        params: &JsonValue::Object(BTreeMap::new()),
        metadata: &admin,
        session: None,
    });
    let server::RpcReply::Result(reply) = reply else {
        panic!("reply must be a result");
    };

    assert_eq!(
        server::from_protocol_json(&reply)["info"]["warnings"][0]["details"]["expected_date"],
        serde_json::Value::Number(1_700_000_000_i64.into())
    );
}

#[test]
fn dispatcher_keeps_admin_unsupported_majority_warning_without_details() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_unsupported_majority_warned(true);
    let dispatcher = BuiltinDispatcher::new(
        rpc::ApplicationServerInfo::new(&app),
        SubscriptionManager::default(),
    );
    let admin = metadata(rpc::RpcRole::Admin);

    let reply = dispatcher.dispatch(RpcRequest {
        method: "server_info",
        params: &JsonValue::Object(BTreeMap::new()),
        metadata: &admin,
        session: None,
    });
    let server::RpcReply::Result(reply) = reply else {
        panic!("reply must be a result");
    };

    let warnings = &server::from_protocol_json(&reply)["info"]["warnings"];
    assert_eq!(warnings.as_array().unwrap().len(), 1);
    assert_eq!(
        warnings[0]["id"],
        serde_json::Value::Number(1001_i64.into())
    );
    assert!(warnings[0].get("details").is_none());
}
