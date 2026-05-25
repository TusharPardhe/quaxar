//! command routing tests part B.

use super::*;

#[test]
fn do_command_ping_admin_shows_role() {
    let access = access();
    let mut session = InfoSub::new(RpcRole::Admin);
    let manager = SubscriptionManager::new();
    let source = FakeServerInfoSource;
    let params = object([]);
    let mut ctx = RpcCommandContext {
        method: "ping",
        params: &params,
        env: &source,
        role: RpcRole::Admin,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: true,
        session: &mut session,
        subscriptions: &manager,
        access: &access,
        remote_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
    };
    let result = do_command(&mut ctx);
    let JsonValue::Object(result) = result else {
        panic!("expected object")
    };
    assert_eq!(
        result.get("role"),
        Some(&JsonValue::String("admin".to_owned()))
    );
    assert_eq!(result.get("unlimited"), Some(&JsonValue::Bool(true)));
}

#[test]
fn do_command_ping_guest_no_role_field() {
    let access = access();
    let mut session = InfoSub::new(RpcRole::Guest);
    let manager = SubscriptionManager::new();
    let source = FakeServerInfoSource;
    let params = object([]);
    let mut ctx = RpcCommandContext {
        method: "ping",
        params: &params,
        env: &source,
        role: RpcRole::Guest,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: false,
        session: &mut session,
        subscriptions: &manager,
        access: &access,
        remote_ip: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 1)),
    };
    let result = do_command(&mut ctx);
    let JsonValue::Object(result) = result else {
        panic!("expected object")
    };
    assert!(!result.contains_key("role"));
    assert!(!result.contains_key("unlimited"));
}

#[test]
fn do_command_ping_proxied_with_forwarded_for_ip() {
    let access = access();
    let mut session = InfoSub::new(RpcRole::Proxy);
    let manager = SubscriptionManager::new();
    let source = FakeServerInfoSource;
    let params = object([]);
    let mut ctx = RpcCommandContext {
        method: "ping",
        params: &params,
        env: &source,
        role: RpcRole::Proxy,
        api_version: 2,
        headers: JsonContextHeaders {
            user: "",
            forwarded_for: "12.34.56.78",
        },
        unlimited: false,
        session: &mut session,
        subscriptions: &manager,
        access: &access,
        remote_ip: IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1)),
    };
    let result = do_command(&mut ctx);
    let JsonValue::Object(result) = result else {
        panic!("expected object")
    };
    assert_eq!(
        result.get("role"),
        Some(&JsonValue::String("proxied".to_owned()))
    );
    assert_eq!(
        result.get("ip"),
        Some(&JsonValue::String("12.34.56.78".to_owned()))
    );
}

#[test]
fn do_command_ping_identified_with_username() {
    let access = access();
    let mut session = InfoSub::new(RpcRole::Identified);
    let manager = SubscriptionManager::new();
    let source = FakeServerInfoSource;
    let params = object([]);
    let mut ctx = RpcCommandContext {
        method: "ping",
        params: &params,
        env: &source,
        role: RpcRole::Identified,
        api_version: 2,
        headers: JsonContextHeaders {
            user: "gateway_user",
            forwarded_for: "87.65.43.21",
        },
        unlimited: true,
        session: &mut session,
        subscriptions: &manager,
        access: &access,
        remote_ip: IpAddr::V4(Ipv4Addr::new(203, 0, 113, 50)),
    };
    let result = do_command(&mut ctx);
    let JsonValue::Object(result) = result else {
        panic!("expected object")
    };
    assert_eq!(
        result.get("role"),
        Some(&JsonValue::String("identified".to_owned()))
    );
    assert_eq!(
        result.get("ip"),
        Some(&JsonValue::String("87.65.43.21".to_owned()))
    );
    assert_eq!(
        result.get("username"),
        Some(&JsonValue::String("gateway_user".to_owned()))
    );
    assert_eq!(result.get("unlimited"), Some(&JsonValue::Bool(true)));
}

#[test]
fn do_command_subscribe_non_array_streams_rejected() {
    let access = access();
    let mut session = InfoSub::new(RpcRole::Guest);
    let manager = SubscriptionManager::new();
    let source = FakeServerInfoSource;
    let params = object([("streams", JsonValue::String("ledger".to_owned()))]);
    let mut ctx = RpcCommandContext {
        method: "subscribe",
        params: &params,
        env: &source,
        role: RpcRole::Guest,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: false,
        session: &mut session,
        subscriptions: &manager,
        access: &access,
        remote_ip: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 10)),
    };
    let result = do_command(&mut ctx);
    let JsonValue::Object(result) = result else {
        panic!("object")
    };
    assert!(result.contains_key("error"));
}

#[test]
fn do_command_subscribe_null_stream_entry_rejected() {
    let access = access();
    let mut session = InfoSub::new(RpcRole::Guest);
    let manager = SubscriptionManager::new();
    let source = FakeServerInfoSource;
    let params = object([("streams", JsonValue::Array(vec![JsonValue::Null]))]);
    let mut ctx = RpcCommandContext {
        method: "subscribe",
        params: &params,
        env: &source,
        role: RpcRole::Guest,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: false,
        session: &mut session,
        subscriptions: &manager,
        access: &access,
        remote_ip: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 11)),
    };
    let result = do_command(&mut ctx);
    let JsonValue::Object(result) = result else {
        panic!("object")
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::StreamMalformed.token().to_owned()
        ))
    );
}

#[test]
fn do_command_subscribe_object_stream_entry_rejected() {
    let access = access();
    let mut session = InfoSub::new(RpcRole::Guest);
    let manager = SubscriptionManager::new();
    let source = FakeServerInfoSource;
    let params = object([(
        "streams",
        JsonValue::Array(vec![JsonValue::Object(Default::default())]),
    )]);
    let mut ctx = RpcCommandContext {
        method: "subscribe",
        params: &params,
        env: &source,
        role: RpcRole::Guest,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: false,
        session: &mut session,
        subscriptions: &manager,
        access: &access,
        remote_ip: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 12)),
    };
    let result = do_command(&mut ctx);
    let JsonValue::Object(result) = result else {
        panic!("object")
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::StreamMalformed.token().to_owned()
        ))
    );
}

#[test]
fn do_command_subscribe_all_valid_stream_names() {
    let access = access();
    let mut session = InfoSub::new(RpcRole::Admin);
    let manager = SubscriptionManager::new();
    let source = FakeServerInfoSource;
    for stream_name in [
        "ledger",
        "transactions",
        "server",
        "validations",
        "manifests",
        "peer_status",
        "consensus",
    ] {
        let params = object([(
            "streams",
            JsonValue::Array(vec![JsonValue::String(stream_name.to_owned())]),
        )]);
        let mut ctx = RpcCommandContext {
            method: "subscribe",
            params: &params,
            env: &source,
            role: RpcRole::Admin,
            api_version: 2,
            headers: JsonContextHeaders::default(),
            unlimited: true,
            session: &mut session,
            subscriptions: &manager,
            access: &access,
            remote_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        };
        let result = do_command(&mut ctx);
        assert_eq!(result, object([]), "stream '{stream_name}' should succeed");
    }
}
