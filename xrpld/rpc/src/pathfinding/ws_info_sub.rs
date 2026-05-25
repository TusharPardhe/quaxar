//! WSInfoSub ported from `xrpld/rpc/detail/WSInfoSub.h`.
//!
//! WebSocket subscription info — tracks user identity, forwarded-for headers,
//! and sends JSON to WS clients. In Rust we use tokio-tungstenite or axum
//! WebSocket types; this provides the abstract interface.

#![allow(dead_code)]

use std::net::IpAddr;
use std::sync::{Arc, Weak};

use protocol::JsonValue;

/// Trait representing a WebSocket session that can receive messages.
pub trait WSSession: Send + Sync {
    /// Get the remote endpoint IP address.
    fn remote_address(&self) -> IpAddr;
    /// Get a request header value by name.
    fn request_header(&self, name: &str) -> Option<String>;
    /// Send a serialized message to the WebSocket client.
    fn send_message(&self, data: Vec<u8>);
}

/// Trait for checking if an IP is allowed through the secure gateway.
pub trait IpAllowCheck {
    fn ip_allowed(&self, addr: &IpAddr) -> bool;
}

/// InfoSub source trait — the subscription manager that owns subscriptions.
pub trait InfoSubSource: Send + Sync {}

/// WebSocket subscription info.
///
/// Tracks user identity (from X-User header) and forwarded-for header
/// when the connection comes through an allowed secure gateway IP.
/// Sends JSON events to the WebSocket client.
pub struct WSInfoSub {
    ws: Weak<dyn WSSession>,
    user: String,
    forwarded_for: String,
}

impl WSInfoSub {
    /// Create a new WSInfoSub from a WebSocket session.
    ///
    /// Extracts X-User and X-Forwarded-For headers only if the remote IP
    /// is in the allowed secure gateway list.
    pub fn new(ws: &Arc<dyn WSSession>, ip_check: &dyn IpAllowCheck) -> Self {
        let mut user = String::new();
        let mut forwarded_for = String::new();

        let remote_addr = ws.remote_address();
        if ip_check.ip_allowed(&remote_addr) {
            if let Some(u) = ws.request_header("X-User") {
                user = u;
            }
            if let Some(fwd) = ws.request_header("X-Forwarded-For") {
                forwarded_for = fwd;
            }
        }

        Self {
            ws: Arc::downgrade(ws),
            user,
            forwarded_for,
        }
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    pub fn forwarded_for(&self) -> &str {
        &self.forwarded_for
    }

    /// Send a JSON value to the WebSocket client.
    /// If the session has been dropped, this is a no-op.
    pub fn send(&self, jv: &JsonValue, _broadcast: bool) {
        let Some(ws) = self.ws.upgrade() else {
            return;
        };
        // Serialize JSON to bytes and send
        let serialized = serialize_json(jv);
        ws.send_message(serialized);
    }

    /// Returns true if the underlying WebSocket session is still alive.
    pub fn is_alive(&self) -> bool {
        self.ws.strong_count() > 0
    }
}

/// Serialize a JsonValue to bytes (UTF-8 JSON string).
fn serialize_json(jv: &JsonValue) -> Vec<u8> {
    // Use the protocol crate's JSON serialization
    protocol::serde_json::to_string(&jv)
        .unwrap_or_default()
        .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Arc;

    struct MockWS {
        addr: IpAddr,
        headers: Vec<(String, String)>,
    }

    impl WSSession for MockWS {
        fn remote_address(&self) -> IpAddr {
            self.addr
        }
        fn request_header(&self, name: &str) -> Option<String> {
            self.headers
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
        fn send_message(&self, _data: Vec<u8>) {}
    }

    struct AllowAll;
    impl IpAllowCheck for AllowAll {
        fn ip_allowed(&self, _addr: &IpAddr) -> bool {
            true
        }
    }

    struct DenyAll;
    impl IpAllowCheck for DenyAll {
        fn ip_allowed(&self, _addr: &IpAddr) -> bool {
            false
        }
    }

    #[test]
    fn extracts_headers_when_allowed() {
        let ws: Arc<dyn WSSession> = Arc::new(MockWS {
            addr: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            headers: vec![
                ("X-User".into(), "admin".into()),
                ("X-Forwarded-For".into(), "10.0.0.1".into()),
            ],
        });
        let sub = WSInfoSub::new(&ws, &AllowAll);
        assert_eq!(sub.user(), "admin");
        assert_eq!(sub.forwarded_for(), "10.0.0.1");
    }

    #[test]
    fn ignores_headers_when_denied() {
        let ws: Arc<dyn WSSession> = Arc::new(MockWS {
            addr: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            headers: vec![
                ("X-User".into(), "admin".into()),
                ("X-Forwarded-For".into(), "10.0.0.1".into()),
            ],
        });
        let sub = WSInfoSub::new(&ws, &DenyAll);
        assert_eq!(sub.user(), "");
        assert_eq!(sub.forwarded_for(), "");
    }
}
