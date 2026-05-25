//! RPC role helpers ported from `xrpld/rpc/Role.h`.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::net::IpAddr;

use ipnet::IpNet;
use protocol::JsonValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Guest,
    User,
    Identified,
    Admin,
    Proxy,
    Forbid,
}

pub fn is_unlimited(role: Role) -> bool {
    matches!(role, Role::Admin | Role::Identified)
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RpcAccessConfig {
    pub admin_nets: Vec<IpNet>,
    pub secure_gateway_nets: Vec<IpNet>,
    pub admin_user: String,
    pub admin_password: String,
}

fn header_value<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn extract_forwarded_ip(field: &str) -> Option<String> {
    fn trim(value: &str) -> &str {
        value.trim_matches([' ', '\r', '\n'])
    }

    let mut value = trim(field);
    if value.is_empty() {
        return None;
    }

    if value.starts_with('"') {
        value = value.strip_prefix('"')?.strip_suffix('"')?;
        value = trim(value);
    }

    if value.is_empty() {
        return None;
    }

    if let Some(stripped) = value.strip_prefix('[') {
        let end = stripped.find(']')?;
        let address = trim(&stripped[..end]);
        return (!address.is_empty()).then(|| address.to_owned());
    }

    let first_non_hex = value
        .char_indices()
        .find(|&(_, ch)| !ch.is_ascii_hexdigit() && ch != ' ');

    let is_ipv6 = match first_non_hex {
        None => true,
        Some((_, ch)) => ch == ':',
    };

    if is_ipv6 {
        return Some(value.to_owned());
    }

    let value = value.split(':').next().unwrap_or(value).trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn forwarded_for_value(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let start = lower.find("for=")? + 4;
    let value = value[start..]
        .split([',', ';'])
        .next()
        .map(str::trim)
        .filter(|entry| !entry.is_empty())?;
    extract_forwarded_ip(value)
}

pub fn forwarded_for(headers: &BTreeMap<String, String>) -> Option<String> {
    header_value(headers, "Forwarded")
        .and_then(forwarded_for_value)
        .or_else(|| {
            header_value(headers, "X-Forwarded-For").and_then(|value| {
                value
                    .split(',')
                    .next()
                    .map(str::trim)
                    .filter(|entry| !entry.is_empty())
                    .and_then(extract_forwarded_ip)
            })
        })
}

fn ip_allowed(remote_ip: IpAddr, nets: &[IpNet]) -> bool {
    nets.iter().any(|net| net.contains(&remote_ip))
}

fn json_string_field<'a>(params: &'a JsonValue, name: &str) -> Option<&'a str> {
    let JsonValue::Object(object) = params else {
        return None;
    };

    match object.get(name) {
        Some(JsonValue::String(value)) => Some(value.as_str()),
        _ => None,
    }
}

pub fn password_unrequired_or_sent_correct(access: &RpcAccessConfig, params: &JsonValue) -> bool {
    let password_required = !access.admin_user.is_empty() || !access.admin_password.is_empty();

    !password_required
        || (json_string_field(params, "admin_user") == Some(access.admin_user.as_str())
            && json_string_field(params, "admin_password") == Some(access.admin_password.as_str()))
}

pub fn is_admin(access: &RpcAccessConfig, params: &JsonValue, remote_ip: IpAddr) -> bool {
    ip_allowed(remote_ip, &access.admin_nets) && password_unrequired_or_sent_correct(access, params)
}

pub fn request_role(
    required: Role,
    access: &RpcAccessConfig,
    params: &JsonValue,
    remote_ip: IpAddr,
    user: &str,
) -> Role {
    if is_admin(access, params, remote_ip) {
        return Role::Admin;
    }

    if required == Role::Admin {
        return Role::Forbid;
    }

    if ip_allowed(remote_ip, &access.secure_gateway_nets) {
        if user.is_empty() {
            Role::Proxy
        } else {
            Role::Identified
        }
    } else {
        Role::Guest
    }
}

pub fn request_is_unlimited(
    required: Role,
    access: &RpcAccessConfig,
    params: &JsonValue,
    remote_ip: IpAddr,
    user: &str,
) -> bool {
    is_unlimited(request_role(required, access, params, remote_ip, user))
}
