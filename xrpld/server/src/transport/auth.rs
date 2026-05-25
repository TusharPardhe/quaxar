use base64::Engine;
use std::net::IpAddr;

use http::HeaderMap;
use ipnet::IpNet;
use protocol::JsonValue;

use rpc::RpcRole as Role;

use crate::session::RequestMetadata;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerAuthConfig {
    pub user: Option<String>,
    pub password: Option<String>,
    pub admin_user: Option<String>,
    pub admin_password: Option<String>,
    pub admin_nets_v4: Vec<IpNet>,
    pub admin_nets_v6: Vec<IpNet>,
    pub secure_gateway_nets_v4: Vec<IpNet>,
    pub secure_gateway_nets_v6: Vec<IpNet>,
}

#[derive(Debug, Clone)]
pub struct ServerAuth {
    pub config: ServerAuthConfig,
}

impl ServerAuth {
    pub fn new(config: ServerAuthConfig) -> Self {
        Self { config }
    }
}

impl Default for ServerAuth {
    fn default() -> Self {
        Self::new(ServerAuthConfig::default())
    }
}

pub fn forwarded_for(headers: &HeaderMap) -> Option<String> {
    fn strip_port(candidate: &str) -> &str {
        let candidate = candidate.trim();
        if candidate.starts_with('[') {
            if let Some(close) = candidate.find(']') {
                return &candidate[1..close];
            }
            return candidate;
        }

        let colon_count = candidate.chars().filter(|c| *c == ':').count();
        if colon_count == 1 {
            candidate.split(':').next().unwrap_or(candidate)
        } else {
            candidate
        }
    }

    if let Some(value) = headers.get("forwarded") {
        let value = value.to_str().ok()?;
        if let Some(index) = value.to_ascii_lowercase().find("for=") {
            let raw = &value[index + 4..];
            let candidate = raw.split([';', ',']).next()?.trim();
            let candidate = candidate.trim_matches('"');
            let candidate = strip_port(candidate);
            if !candidate.is_empty() {
                return Some(candidate.to_owned());
            }
        }
    }

    if let Some(value) = headers.get("x-forwarded-for") {
        let value = value.to_str().ok()?;
        let candidate = value.split(',').next()?.trim();
        let candidate = strip_port(candidate);
        if !candidate.is_empty() {
            return Some(candidate.to_owned());
        }
    }

    None
}

pub fn ip_allowed(remote_ip: IpAddr, nets4: &[IpNet], nets6: &[IpNet]) -> bool {
    nets4
        .iter()
        .chain(nets6.iter())
        .any(|net| net.contains(&remote_ip))
}

pub fn authorized_http(config: &ServerAuthConfig, headers: &HeaderMap) -> bool {
    let Some(expected_user) = config.user.as_deref() else {
        return true;
    };
    let Some(expected_password) = config.password.as_deref() else {
        return true;
    };

    let Some(header) = headers.get(http::header::AUTHORIZATION) else {
        return false;
    };
    let Ok(header) = header.to_str() else {
        return false;
    };
    let Some(encoded) = header.strip_prefix("Basic ") else {
        return false;
    };
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded.trim()) else {
        return false;
    };
    let Ok(decoded) = String::from_utf8(decoded) else {
        return false;
    };
    let Some((user, password)) = decoded.split_once(':') else {
        return false;
    };

    user == expected_user && password == expected_password
}

pub fn password_unrequired_or_sent_correct(config: &ServerAuthConfig, params: &JsonValue) -> bool {
    let password_required = config.admin_user.is_some() || config.admin_password.is_some();
    if !password_required {
        return true;
    }

    let JsonValue::Object(object) = params else {
        return false;
    };

    let user_ok = object
        .get("admin_user")
        .and_then(|value| match value {
            JsonValue::String(text) => Some(text == config.admin_user.as_deref().unwrap_or("")),
            _ => None,
        })
        .unwrap_or(false);
    let password_ok = object
        .get("admin_password")
        .and_then(|value| match value {
            JsonValue::String(text) => Some(text == config.admin_password.as_deref().unwrap_or("")),
            _ => None,
        })
        .unwrap_or(false);

    user_ok && password_ok
}

pub fn is_admin(config: &ServerAuthConfig, params: &JsonValue, remote_ip: IpAddr) -> bool {
    ip_allowed(remote_ip, &config.admin_nets_v4, &config.admin_nets_v6)
        && password_unrequired_or_sent_correct(config, params)
}

pub fn request_role(
    required: Role,
    auth: &ServerAuth,
    metadata: &RequestMetadata,
    params: &JsonValue,
    user: &str,
) -> Role {
    let role = request_role_inner(required, auth, metadata, params, user);
    tracing::debug!(target: "server", role = ?role, ip = %metadata.remote_addr.ip(), "RPC role determined");
    role
}

fn request_role_inner(
    required: Role,
    auth: &ServerAuth,
    metadata: &RequestMetadata,
    params: &JsonValue,
    user: &str,
) -> Role {
    if is_admin(&auth.config, params, metadata.remote_addr.ip()) {
        return Role::Admin;
    }

    if required == Role::Admin {
        return Role::Forbid;
    }

    if ip_allowed(
        metadata.remote_addr.ip(),
        &auth.config.secure_gateway_nets_v4,
        &auth.config.secure_gateway_nets_v6,
    ) {
        if user.is_empty() {
            Role::Proxy
        } else {
            Role::Identified
        }
    } else {
        Role::Guest
    }
}

#[cfg(test)]
mod tests {
    use super::{ServerAuthConfig, authorized_http};
    use http::{HeaderMap, HeaderValue, header};

    #[test]
    fn authorized_http_matches_basic_auth() {
        let config = ServerAuthConfig {
            user: Some("rpc".to_owned()),
            password: Some("secret".to_owned()),
            ..ServerAuthConfig::default()
        };

        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Basic cnBjOnNlY3JldA=="),
        );
        assert!(authorized_http(&config, &headers));

        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Basic cnBjOndyb25n"),
        );
        assert!(!authorized_http(&config, &headers));
    }
}
