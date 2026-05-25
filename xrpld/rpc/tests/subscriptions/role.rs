//! Tests for RPC role resolution, forwarded-for header parsing, and admin auth.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;

use ipnet::IpNet;
use protocol::JsonValue;
use rpc::{RpcAccessConfig, RpcRole, forwarded_for, request_is_unlimited, request_role};

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn access() -> RpcAccessConfig {
    RpcAccessConfig {
        admin_nets: vec![IpNet::from_str("127.0.0.0/8").expect("admin net")],
        secure_gateway_nets: vec![IpNet::from_str("203.0.113.0/24").expect("gateway net")],
        admin_user: String::new(),
        admin_password: String::new(),
    }
}

#[test]
fn forwarded_for_header_trimming_rules() {
    let headers = BTreeMap::from([(
        "Forwarded".to_owned(),
        "for=\"[2001:db8::1]:1234\", for=198.51.100.9".to_owned(),
    )]);

    assert_eq!(forwarded_for(&headers), Some("2001:db8::1".to_owned()));
}

#[test]
fn forwarded_for_falls_back_to_x_forwarded_for() {
    let headers = BTreeMap::from([(
        "X-Forwarded-For".to_owned(),
        " 198.51.100.42:4789, 203.0.113.7".to_owned(),
    )]);

    assert_eq!(forwarded_for(&headers), Some("198.51.100.42".to_owned()));
}

#[test]
fn request_role_admin_and_gateway_rules() {
    let params = object([]);
    let access = access();
    let admin = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
    let gateway = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 42));
    let guest = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7));

    assert_eq!(
        request_role(RpcRole::Guest, &access, &params, admin, ""),
        RpcRole::Admin
    );
    assert_eq!(
        request_role(RpcRole::Guest, &access, &params, gateway, "alice"),
        RpcRole::Identified
    );
    assert_eq!(
        request_role(RpcRole::Guest, &access, &params, gateway, ""),
        RpcRole::Proxy
    );
    assert_eq!(
        request_role(RpcRole::Admin, &access, &params, guest, ""),
        RpcRole::Forbid
    );
    assert_eq!(
        request_role(RpcRole::Guest, &access, &params, guest, ""),
        RpcRole::Guest
    );
}

#[test]
fn request_is_unlimited_role_contract() {
    let params = object([]);
    let access = access();

    assert!(request_is_unlimited(
        RpcRole::Guest,
        &access,
        &params,
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        ""
    ));
    assert!(request_is_unlimited(
        RpcRole::Guest,
        &access,
        &params,
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 8)),
        "alice"
    ));
    assert!(!request_is_unlimited(
        RpcRole::Admin,
        &access,
        &params,
        IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)),
        ""
    ));
}

#[test]
fn admin_password_gate_requires_both_credentials() {
    let access = RpcAccessConfig {
        admin_nets: vec![IpNet::from_str("127.0.0.0/8").expect("admin net")],
        secure_gateway_nets: vec![],
        admin_user: "rpc".to_owned(),
        admin_password: "secret".to_owned(),
    };
    let params = object([
        ("admin_user", JsonValue::String("rpc".to_owned())),
        ("admin_password", JsonValue::String("secret".to_owned())),
    ]);

    assert!(rpc::password_unrequired_or_sent_correct(&access, &params));
    assert!(rpc::is_admin(
        &access,
        &params,
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
    ));

    let missing_password = object([("admin_user", JsonValue::String("rpc".to_owned()))]);
    assert!(!rpc::password_unrequired_or_sent_correct(
        &access,
        &missing_password
    ));
    assert!(!rpc::is_admin(
        &access,
        &missing_password,
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
    ));
}

#[test]
fn forwarded_for_extracts_first_ip_from_comma_list() {
    let mut headers = BTreeMap::new();
    headers.insert(
        "x-forwarded-for".to_owned(),
        "87.65.43.21, 44.33.22.11".to_owned(),
    );
    let result = forwarded_for(&headers);
    assert_eq!(result, Some("87.65.43.21".to_owned()));
}

#[test]
fn forwarded_for_strips_port_from_ip() {
    let mut headers = BTreeMap::new();
    headers.insert(
        "x-forwarded-for".to_owned(),
        "87.65.43.21:47011, 44.33.22.11".to_owned(),
    );
    let result = forwarded_for(&headers);
    assert_eq!(result, Some("87.65.43.21".to_owned()));
}

#[test]
fn forwarded_for_empty_headers() {
    let headers = BTreeMap::new();
    let result = forwarded_for(&headers);
    assert_eq!(result, None);
}

#[test]
fn forwarded_for_rfc7239_format() {
    let mut headers = BTreeMap::new();
    headers.insert("forwarded".to_owned(), "for=88.77.66.55".to_owned());
    let result = forwarded_for(&headers);
    assert_eq!(result, Some("88.77.66.55".to_owned()));
}

#[test]
fn forwarded_for_rfc7239_multiple_for() {
    let mut headers = BTreeMap::new();
    headers.insert(
        "forwarded".to_owned(),
        "what=where;for=55.66.77.88;for=nobody;".to_owned(),
    );
    let result = forwarded_for(&headers);
    assert_eq!(result, Some("55.66.77.88".to_owned()));
}
