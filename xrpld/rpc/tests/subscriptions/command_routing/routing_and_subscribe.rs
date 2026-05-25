//! command routing tests part A.

use super::*;

#[test]
fn do_command_routes_ping_status_methods_subscribe_and_unsubscribe() {
    let params = object([(
        "streams",
        JsonValue::Array(vec![JsonValue::String("transactions".to_owned())]),
    )]);
    let access = access();
    let mut session = InfoSub::new(RpcRole::Guest);
    let manager = SubscriptionManager::new();
    let source = FakeServerInfoSource;

    let subscribe_params = object([(
        "streams",
        JsonValue::Array(vec![JsonValue::String("transactions".to_owned())]),
    )]);
    let ping_params = object([]);
    {
        let mut fee_context = RpcCommandContext {
            method: "fee",
            params: &ping_params,
            env: &source,
            role: RpcRole::Identified,
            api_version: 2,
            headers: JsonContextHeaders::default(),
            unlimited: false,
            session: &mut session,
            subscriptions: &manager,
            access: &access,
            remote_ip: IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)),
        };
        assert_eq!(
            do_command(&mut fee_context),
            object([("fee", JsonValue::String("ok".to_owned()))])
        );
    }
    {
        let mut ledger_current_context = RpcCommandContext {
            method: "ledger_current",
            params: &ping_params,
            env: &source,
            role: RpcRole::Identified,
            api_version: 2,
            headers: JsonContextHeaders::default(),
            unlimited: false,
            session: &mut session,
            subscriptions: &manager,
            access: &access,
            remote_ip: IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)),
        };
        assert_eq!(
            do_command(&mut ledger_current_context),
            object([("ledger_current_index", JsonValue::Unsigned(991))])
        );
    }
    {
        let mut ledger_closed_context = RpcCommandContext {
            method: "ledger_closed",
            params: &ping_params,
            env: &source,
            role: RpcRole::Identified,
            api_version: 2,
            headers: JsonContextHeaders::default(),
            unlimited: false,
            session: &mut session,
            subscriptions: &manager,
            access: &access,
            remote_ip: IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)),
        };
        assert_eq!(
            do_command(&mut ledger_closed_context),
            object([
                ("ledger_hash", JsonValue::String(format!("{:064X}", 0xABCD))),
                ("ledger_index", JsonValue::Unsigned(990))
            ])
        );
    }
    {
        let mut ping_context = RpcCommandContext {
            method: "ping",
            params: &ping_params,
            env: &source,
            role: RpcRole::Identified,
            api_version: 2,
            headers: JsonContextHeaders {
                user: "alice",
                forwarded_for: "203.0.113.9",
            },
            unlimited: true,
            session: &mut session,
            subscriptions: &manager,
            access: &access,
            remote_ip: IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)),
        };
        assert_eq!(
            do_command(&mut ping_context),
            object([
                ("ip", JsonValue::String("203.0.113.9".to_owned())),
                ("role", JsonValue::String("identified".to_owned())),
                ("unlimited", JsonValue::Bool(true)),
                ("username", JsonValue::String("alice".to_owned()))
            ])
        );
    }
    {
        let mut info_context = RpcCommandContext {
            method: "server_info",
            params: &params,
            env: &source,
            role: RpcRole::Admin,
            api_version: 2,
            headers: JsonContextHeaders::default(),
            unlimited: false,
            session: &mut session,
            subscriptions: &manager,
            access: &access,
            remote_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        };
        assert_eq!(
            do_command(&mut info_context),
            object([(
                "info",
                object([
                    ("admin", JsonValue::Bool(true)),
                    ("counters", JsonValue::Bool(false)),
                    ("human", JsonValue::Bool(true))
                ])
            )])
        );
    }
    {
        let mut subscribe_context = RpcCommandContext {
            method: "subscribe",
            params: &subscribe_params,
            env: &source,
            role: RpcRole::Guest,
            api_version: 2,
            headers: JsonContextHeaders::default(),
            unlimited: false,
            session: &mut session,
            subscriptions: &manager,
            access: &access,
            remote_ip: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)),
        };
        assert_eq!(do_command(&mut subscribe_context), object([]));
    }
    assert!(session.is_subscribed(SubscriptionStream::Transactions));
    {
        let mut unsubscribe_context = RpcCommandContext {
            method: "unsubscribe",
            params: &subscribe_params,
            env: &source,
            role: RpcRole::Guest,
            api_version: 2,
            headers: JsonContextHeaders::default(),
            unlimited: false,
            session: &mut session,
            subscriptions: &manager,
            access: &access,
            remote_ip: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)),
        };
        assert_eq!(do_command(&mut unsubscribe_context), object([]));
    }
    assert!(!session.is_subscribed(SubscriptionStream::Transactions));

    let bad_stream_params = object([("streams", JsonValue::Array(vec![JsonValue::Unsigned(1)]))]);
    {
        let mut bad_stream_context = RpcCommandContext {
            method: "subscribe",
            params: &bad_stream_params,
            env: &source,
            role: RpcRole::Guest,
            api_version: 2,
            headers: JsonContextHeaders::default(),
            unlimited: false,
            session: &mut session,
            subscriptions: &manager,
            access: &access,
            remote_ip: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)),
        };
        let result = do_command(&mut bad_stream_context);
        let JsonValue::Object(result) = result else {
            panic!("expected error object")
        };
        assert_eq!(
            result.get("error"),
            Some(&JsonValue::String(
                RpcErrorCode::StreamMalformed.token().to_owned()
            ))
        );
    }
}

#[test]
fn do_command_subscribe_multiple_streams() {
    let access = access();
    let mut session = InfoSub::new(RpcRole::Guest);
    let manager = SubscriptionManager::new();
    let source = FakeServerInfoSource;
    let params = object([(
        "streams",
        JsonValue::Array(vec![
            JsonValue::String("ledger".to_owned()),
            JsonValue::String("transactions".to_owned()),
            JsonValue::String("server".to_owned()),
        ]),
    )]);
    {
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
            remote_ip: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 1)),
        };
        assert_eq!(do_command(&mut ctx), object([]));
    }
    assert!(session.is_subscribed(SubscriptionStream::Ledger));
    assert!(session.is_subscribed(SubscriptionStream::Transactions));
    assert!(session.is_subscribed(SubscriptionStream::Server));
}

#[test]
fn do_command_subscribe_invalid_stream_name() {
    let access = access();
    let mut session = InfoSub::new(RpcRole::Guest);
    let manager = SubscriptionManager::new();
    let source = FakeServerInfoSource;
    let params = object([(
        "streams",
        JsonValue::Array(vec![JsonValue::String("invalid_stream".to_owned())]),
    )]);
    {
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
            remote_ip: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 2)),
        };
        let result = do_command(&mut ctx);
        let JsonValue::Object(result) = result else {
            panic!("expected object")
        };
        assert_eq!(
            result.get("error"),
            Some(&JsonValue::String(
                RpcErrorCode::StreamMalformed.token().to_owned()
            ))
        );
    }
}

#[test]
fn do_command_unsubscribe_without_prior_subscribe() {
    let access = access();
    let mut session = InfoSub::new(RpcRole::Guest);
    let manager = SubscriptionManager::new();
    let source = FakeServerInfoSource;
    let params = object([(
        "streams",
        JsonValue::Array(vec![JsonValue::String("ledger".to_owned())]),
    )]);
    assert!(!session.is_subscribed(SubscriptionStream::Ledger));
    {
        let mut ctx = RpcCommandContext {
            method: "unsubscribe",
            params: &params,
            env: &source,
            role: RpcRole::Guest,
            api_version: 2,
            headers: JsonContextHeaders::default(),
            unlimited: false,
            session: &mut session,
            subscriptions: &manager,
            access: &access,
            remote_ip: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 3)),
        };
        let result = do_command(&mut ctx);
        assert_eq!(result, object([]));
    }
    assert!(!session.is_subscribed(SubscriptionStream::Ledger));
}

#[test]
fn do_command_unknown_method_returns_error() {
    let access = access();
    let mut session = InfoSub::new(RpcRole::Guest);
    let manager = SubscriptionManager::new();
    let source = FakeServerInfoSource;
    let params = object([]);
    {
        let mut ctx = RpcCommandContext {
            method: "nonexistent_method",
            params: &params,
            env: &source,
            role: RpcRole::Guest,
            api_version: 2,
            headers: JsonContextHeaders::default(),
            unlimited: false,
            session: &mut session,
            subscriptions: &manager,
            access: &access,
            remote_ip: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 4)),
        };
        let result = do_command(&mut ctx);
        let JsonValue::Object(result) = result else {
            panic!("expected object")
        };
        assert!(result.contains_key("error"));
        assert_eq!(
            result.get("error"),
            Some(&JsonValue::String("unknownCmd".to_owned()))
        );
    }
}

#[test]
fn do_command_server_state_routes_correctly() {
    let access = access();
    let mut session = InfoSub::new(RpcRole::Admin);
    let manager = SubscriptionManager::new();
    let source = FakeServerInfoSource;
    let params = object([]);
    {
        let mut ctx = RpcCommandContext {
            method: "server_state",
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
        assert!(result.contains_key("state"));
    }
}

#[test]
fn do_command_subscribe_empty_streams_array() {
    let access = access();
    let mut session = InfoSub::new(RpcRole::Guest);
    let manager = SubscriptionManager::new();
    let source = FakeServerInfoSource;
    let params = object([("streams", JsonValue::Array(vec![]))]);
    {
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
            remote_ip: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 5)),
        };
        assert_eq!(do_command(&mut ctx), object([]));
    }
}
