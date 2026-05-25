//! Tests for the ping RPC handler.

use std::collections::BTreeMap;

use protocol::JsonValue;
use rpc::{JsonContext, JsonContextHeaders, RpcRole, do_ping};

fn object() -> JsonValue {
    JsonValue::Object(BTreeMap::new())
}

#[test]
fn ping_matches_admin_shape() {
    let params = object();
    let env = ();
    let result = do_ping(&JsonContext {
        params: &params,
        env: &env,
        role: RpcRole::Admin,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: false,
    });

    assert_eq!(
        result,
        JsonValue::Object(BTreeMap::from([(
            "role".to_owned(),
            JsonValue::String("admin".to_owned()),
        )]))
    );
}

#[test]
fn ping_matches_identified_shape() {
    let params = object();
    let env = ();
    let result = do_ping(&JsonContext {
        params: &params,
        env: &env,
        role: RpcRole::Identified,
        api_version: 2,
        headers: JsonContextHeaders {
            user: "alice",
            forwarded_for: "203.0.113.9",
        },
        unlimited: true,
    });

    assert_eq!(
        result,
        JsonValue::Object(BTreeMap::from([
            ("ip".to_owned(), JsonValue::String("203.0.113.9".to_owned())),
            (
                "role".to_owned(),
                JsonValue::String("identified".to_owned())
            ),
            ("unlimited".to_owned(), JsonValue::Bool(true)),
            ("username".to_owned(), JsonValue::String("alice".to_owned())),
        ]))
    );
}

#[test]
fn ping_matches_proxy_shape() {
    let params = object();
    let env = ();
    let result = do_ping(&JsonContext {
        params: &params,
        env: &env,
        role: RpcRole::Proxy,
        api_version: 2,
        headers: JsonContextHeaders {
            user: "",
            forwarded_for: "198.51.100.42",
        },
        unlimited: false,
    });

    assert_eq!(
        result,
        JsonValue::Object(BTreeMap::from([
            (
                "ip".to_owned(),
                JsonValue::String("198.51.100.42".to_owned())
            ),
            ("role".to_owned(), JsonValue::String("proxied".to_owned())),
        ]))
    );
}

#[test]
fn ping_keeps_guest_empty_without_unlimited_flag() {
    let params = object();
    let env = ();
    let result = do_ping(&JsonContext {
        params: &params,
        env: &env,
        role: RpcRole::Guest,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: false,
    });

    assert_eq!(result, JsonValue::Object(BTreeMap::new()));
}

#[test]
fn ping_user_role_returns_empty_like_guest() {
    let params = object();
    let env = ();
    let result = do_ping(&JsonContext {
        params: &params,
        env: &env,
        role: RpcRole::User,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: false,
    });

    // User without unlimited returns empty object (same as guest)
    assert_eq!(result, JsonValue::Object(BTreeMap::new()));
}

#[test]
fn ping_unlimited_flag_shown_when_true() {
    let params = object();
    let env = ();
    let result = do_ping(&JsonContext {
        params: &params,
        env: &env,
        role: RpcRole::Admin,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: true,
    });

    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("unlimited"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        result.get("role"),
        Some(&JsonValue::String("admin".to_owned()))
    );
}
