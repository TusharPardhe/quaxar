//! XRPL HTTP Upgrade handshake helpers.

use std::net::IpAddr;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use basics::base_uint::Uint256;
use basics::base64::base64_decode;
use basics::time::chrono::EPOCH_OFFSET_SECONDS;
use http::header::{CONNECTION, HeaderName, HeaderValue, SERVER, UPGRADE, USER_AGENT};
use http::{HeaderMap, Method, Request, Response, StatusCode, Version};
use protocol::{
    KeyType, PublicKey, parse_base58_node_public, sha512_digest, sha512_half, verify_digest,
};

use crate::protocol_version::{ProtocolVersion, supported_protocol_versions};

pub const FEATURE_COMPR: &str = "compr";
pub const FEATURE_VPRR: &str = "vprr";
pub const FEATURE_TXRR: &str = "txrr";
pub const FEATURE_LEDGER_REPLAY: &str = "ledgerreplay";
const X_PROTOCOL_CTL: &str = "X-Protocol-Ctl";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakeContext {
    pub network_id: Option<u32>,
    pub network_time: i64,
    pub public_key: String,
    pub session_signature: String,
    pub instance_cookie: u64,
    pub server_domain: Option<String>,
    pub remote_ip: Option<IpAddr>,
    pub local_ip: Option<IpAddr>,
    pub closed_ledger: Option<String>,
    pub previous_ledger: Option<String>,
}

impl HandshakeContext {
    pub fn new(public_key: String, session_signature: String, instance_cookie: u64) -> Self {
        let unix_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let network_time = unix_time.saturating_sub(EPOCH_OFFSET_SECONDS);
        Self {
            network_id: None,
            network_time,
            public_key,
            session_signature,
            instance_cookie,
            server_domain: None,
            remote_ip: None,
            local_ip: None,
            closed_ledger: None,
            previous_ledger: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakeVerificationContext {
    pub shared_value: Uint256,
    pub network_id: Option<u32>,
    pub local_public_key: Option<PublicKey>,
    pub public_ip: Option<IpAddr>,
    pub remote_ip: IpAddr,
    pub clock_tolerance: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakePeer {
    pub public_key: PublicKey,
    pub server_domain: Option<String>,
    pub closed_ledger: Option<String>,
    pub previous_ledger: Option<String>,
}

pub fn get_feature_value(headers: &HeaderMap, feature: &str) -> Option<String> {
    let all_features = headers.get(X_PROTOCOL_CTL)?.to_str().ok()?;
    for entry in all_features.split(';') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let (name, value) = entry.split_once('=')?;
        if name == feature {
            return Some(value.trim().to_owned());
        }
    }
    None
}

pub fn is_feature_value(headers: &HeaderMap, feature: &str, value: &str) -> bool {
    get_feature_value(headers, feature)
        .is_some_and(|feature_value| feature_value.split(',').any(|item| item.trim() == value))
}

pub fn feature_enabled(headers: &HeaderMap, feature: &str) -> bool {
    is_feature_value(headers, feature, "1")
}

pub fn make_features_request_header(
    compr_enabled: bool,
    ledger_replay_enabled: bool,
    tx_reduce_relay_enabled: bool,
    vp_reduce_relay_enabled: bool,
) -> String {
    let mut header = String::new();
    if compr_enabled {
        header.push_str("compr=lz4;");
    }
    if ledger_replay_enabled {
        header.push_str("ledgerreplay=1;");
    }
    if tx_reduce_relay_enabled {
        header.push_str("txrr=1;");
    }
    if vp_reduce_relay_enabled {
        header.push_str("vprr=1;");
    }
    header
}

pub fn make_features_response_header(
    request: &Request<()>,
    compr_enabled: bool,
    ledger_replay_enabled: bool,
    tx_reduce_relay_enabled: bool,
    vp_reduce_relay_enabled: bool,
) -> String {
    let headers = request.headers();
    let mut header = String::new();
    if compr_enabled && is_feature_value(headers, FEATURE_COMPR, "lz4") {
        header.push_str("compr=lz4;");
    }
    if ledger_replay_enabled && feature_enabled(headers, FEATURE_LEDGER_REPLAY) {
        header.push_str("ledgerreplay=1;");
    }
    if tx_reduce_relay_enabled && feature_enabled(headers, FEATURE_TXRR) {
        header.push_str("txrr=1;");
    }
    if vp_reduce_relay_enabled && feature_enabled(headers, FEATURE_VPRR) {
        header.push_str("vprr=1;");
    }
    header
}

pub fn make_request(
    crawl_public: bool,
    compr_enabled: bool,
    ledger_replay_enabled: bool,
    tx_reduce_relay_enabled: bool,
    vp_reduce_relay_enabled: bool,
) -> Request<()> {
    let mut request = Request::builder()
        .method(Method::GET)
        .uri("/")
        .version(Version::HTTP_11)
        .header(USER_AGENT, HeaderValue::from_static("xrpld-rust/overlay"))
        .header(UPGRADE, supported_protocol_versions())
        .header(CONNECTION, HeaderValue::from_static("Upgrade"))
        .header("Connect-As", HeaderValue::from_static("Peer"))
        .header("Crawl", if crawl_public { "public" } else { "private" })
        .body(())
        .expect("overlay request builder");
    set_header(
        request.headers_mut(),
        X_PROTOCOL_CTL,
        &make_features_request_header(
            compr_enabled,
            ledger_replay_enabled,
            tx_reduce_relay_enabled,
            vp_reduce_relay_enabled,
        ),
    );
    request
}

#[allow(clippy::too_many_arguments)]
pub fn make_response(
    crawl_public: bool,
    request: &Request<()>,
    shared_context: &HandshakeContext,
    protocol: ProtocolVersion,
    compr_enabled: bool,
    ledger_replay_enabled: bool,
    tx_reduce_relay_enabled: bool,
    vp_reduce_relay_enabled: bool,
) -> Response<()> {
    let mut response = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .version(request.version())
        .header(CONNECTION, HeaderValue::from_static("Upgrade"))
        .header(UPGRADE, protocol.to_string())
        .header("Connect-As", HeaderValue::from_static("Peer"))
        .header(SERVER, HeaderValue::from_static("xrpld-rust/overlay"))
        .header("Crawl", if crawl_public { "public" } else { "private" })
        .body(())
        .expect("overlay response builder");
    set_header(
        response.headers_mut(),
        X_PROTOCOL_CTL,
        &make_features_response_header(
            request,
            compr_enabled,
            ledger_replay_enabled,
            tx_reduce_relay_enabled,
            vp_reduce_relay_enabled,
        ),
    );
    build_handshake(response.headers_mut(), shared_context);
    response
}

pub fn build_handshake(headers: &mut HeaderMap, context: &HandshakeContext) {
    if let Some(network_id) = context.network_id {
        set_header(headers, "Network-ID", &network_id.to_string());
    }
    set_header(headers, "Network-Time", &context.network_time.to_string());
    set_header(headers, "Public-Key", &context.public_key);
    set_header(headers, "Session-Signature", &context.session_signature);
    set_header(
        headers,
        "Instance-Cookie",
        &context.instance_cookie.to_string(),
    );
    if let Some(server_domain) = &context.server_domain {
        set_header(headers, "Server-Domain", server_domain);
    }
    if let Some(remote_ip) = context.remote_ip
        && is_public_ip(remote_ip)
    {
        set_header(headers, "Remote-IP", &remote_ip.to_string());
    }
    if let Some(local_ip) = context.local_ip
        && !is_unspecified_ip(local_ip)
    {
        set_header(headers, "Local-IP", &local_ip.to_string());
    }
    if let Some(closed_ledger) = &context.closed_ledger {
        set_header(headers, "Closed-Ledger", closed_ledger);
    }
    if let Some(previous_ledger) = &context.previous_ledger {
        set_header(headers, "Previous-Ledger", previous_ledger);
    }
}

pub fn make_shared_value_from_finished_messages(
    local_finished: &[u8],
    peer_finished: &[u8],
) -> Option<Uint256> {
    const SSL_MINIMUM_FINISHED_LENGTH: usize = 12;

    if local_finished.len() < SSL_MINIMUM_FINISHED_LENGTH
        || peer_finished.len() < SSL_MINIMUM_FINISHED_LENGTH
    {
        return None;
    }

    let local_cookie = sha512_digest(local_finished);
    let peer_cookie = sha512_digest(peer_finished);
    let mixed: [u8; 64] = std::array::from_fn(|index| local_cookie[index] ^ peer_cookie[index]);
    if mixed.iter().all(|byte| *byte == 0) {
        return None;
    }

    Some(sha512_half(mixed))
}

pub fn verify_handshake(
    headers: &HeaderMap,
    context: &HandshakeVerificationContext,
) -> Result<HandshakePeer, String> {
    tracing::debug!(target: "overlay", "Handshake initiated");

    if let Some(server_domain) = header_value(headers, "Server-Domain")
        && (server_domain.trim().is_empty() || !server_domain.contains('.'))
    {
        tracing::warn!(target: "overlay", reason = "Invalid server domain", "Handshake failed");
        return Err("Invalid server domain".to_owned());
    }

    if let Some(network_id) = header_value(headers, "Network-ID") {
        let parsed = network_id
            .parse::<u32>()
            .map_err(|_| {
                tracing::warn!(target: "overlay", reason = "Invalid peer network identifier", "Handshake failed");
                "Invalid peer network identifier".to_owned()
            })?;
        if context
            .network_id
            .is_some_and(|expected| expected != parsed)
        {
            tracing::warn!(target: "overlay", reason = "Peer is on a different network", "Handshake failed");
            return Err("Peer is on a different network".to_owned());
        }
    }

    if let Some(network_time) = header_value(headers, "Network-Time") {
        let network_time = network_time
            .parse::<i64>()
            .map_err(|_| {
                tracing::warn!(target: "overlay", reason = "Invalid peer clock timestamp", "Handshake failed");
                "Invalid peer clock timestamp".to_owned()
            })?;
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let now = now_unix.saturating_sub(EPOCH_OFFSET_SECONDS);
        let offset = (network_time - now).unsigned_abs();
        if offset > context.clock_tolerance.as_secs() {
            tracing::warn!(target: "overlay", reason = "Peer clock is too far off", "Handshake failed");
            return Err("Peer clock is too far off".to_owned());
        }
    }

    let public_key_text = header_value(headers, "Public-Key").ok_or_else(|| {
        tracing::warn!(target: "overlay", reason = "Bad node public key", "Handshake failed");
        "Bad node public key".to_owned()
    })?;
    let public_key_bytes = parse_base58_node_public(&public_key_text).ok_or_else(|| {
        tracing::warn!(target: "overlay", reason = "Bad node public key", "Handshake failed");
        "Bad node public key".to_owned()
    })?;
    let public_key = PublicKey::from_slice(&public_key_bytes).map_err(|_| {
        tracing::warn!(target: "overlay", reason = "Bad node public key", "Handshake failed");
        "Bad node public key".to_owned()
    })?;
    if public_key.key_type() != Some(KeyType::Secp256k1) {
        tracing::warn!(target: "overlay", reason = "Unsupported public key type", "Handshake failed");
        return Err("Unsupported public key type".to_owned());
    }

    let signature = header_value(headers, "Session-Signature")
        .ok_or_else(|| {
            tracing::warn!(target: "overlay", reason = "No session signature specified", "Handshake failed");
            "No session signature specified".to_owned()
        })?;
    let signature = base64_decode(&signature);
    if !verify_digest(&public_key, context.shared_value, &signature, false) {
        tracing::warn!(target: "overlay", reason = "Failed to verify session", "Handshake failed");
        return Err("Failed to verify session".to_owned());
    }
    if context
        .local_public_key
        .is_some_and(|local| local == public_key)
    {
        tracing::warn!(target: "overlay", reason = "Self connection", "Handshake failed");
        return Err("Self connection".to_owned());
    }

    if let Some(local_ip) = header_value(headers, "Local-IP") {
        let local_ip = IpAddr::from_str(&local_ip).map_err(|_| {
            tracing::warn!(target: "overlay", reason = "Invalid Local-IP", "Handshake failed");
            "Invalid Local-IP".to_owned()
        })?;
        if is_public_ip(context.remote_ip) && context.remote_ip != local_ip {
            let reason = format!(
                "Incorrect Local-IP: {} instead of {}",
                context.remote_ip, local_ip
            );
            tracing::warn!(target: "overlay", %reason, "Handshake failed");
            return Err(reason);
        }
    }

    if let Some(remote_ip) = header_value(headers, "Remote-IP") {
        let remote_ip = IpAddr::from_str(&remote_ip).map_err(|_| {
            tracing::warn!(target: "overlay", reason = "Invalid Remote-IP", "Handshake failed");
            "Invalid Remote-IP".to_owned()
        })?;
        if is_public_ip(context.remote_ip)
            && context.public_ip.is_some()
            && context.public_ip != Some(remote_ip)
        {
            let reason = format!(
                "Incorrect Remote-IP: {} instead of {}",
                context.public_ip.expect("checked is_some"),
                remote_ip
            );
            tracing::warn!(target: "overlay", %reason, "Handshake failed");
            return Err(reason);
        }
    }

    tracing::info!(target: "overlay", "Handshake complete");
    Ok(HandshakePeer {
        public_key,
        server_domain: header_value(headers, "Server-Domain"),
        closed_ledger: header_value(headers, "Closed-Ledger"),
        previous_ledger: header_value(headers, "Previous-Ledger"),
    })
}

pub fn serialize_request(request: &Request<()>) -> Vec<u8> {
    let mut bytes = format!(
        "{} {} {}\r\n",
        request.method(),
        request.uri(),
        http_version_text(request.version())
    )
    .into_bytes();
    bytes.extend(serialize_headers(request.headers()));
    bytes
}

pub fn serialize_response(response: &Response<()>) -> Vec<u8> {
    let reason = response.status().canonical_reason().unwrap_or("");
    let mut bytes = format!(
        "{} {} {}\r\n",
        http_version_text(response.version()),
        response.status().as_u16(),
        reason
    )
    .into_bytes();
    bytes.extend(serialize_headers(response.headers()));
    bytes
}

pub fn parse_http_request(bytes: &[u8]) -> Result<Request<()>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "request is not utf8".to_owned())?;
    let (head, _) = split_head(text)?;
    let mut lines = head.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| "missing request line".to_owned())?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| "missing method".to_owned())?
        .parse::<Method>()
        .map_err(|_| "invalid method".to_owned())?;
    let target = parts.next().ok_or_else(|| "missing target".to_owned())?;
    let version = parse_http_version(parts.next().ok_or_else(|| "missing version".to_owned())?)?;
    let mut builder = Request::builder()
        .method(method)
        .uri(target)
        .version(version);
    let headers = builder
        .headers_mut()
        .ok_or_else(|| "request headers unavailable".to_owned())?;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| "malformed request header".to_owned())?;
        set_header(headers, name.trim(), value.trim());
    }
    builder.body(()).map_err(|error| error.to_string())
}

pub fn parse_http_response(bytes: &[u8]) -> Result<Response<()>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "response is not utf8".to_owned())?;
    let (head, _) = split_head(text)?;
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| "missing status line".to_owned())?;
    let mut parts = status_line.split_whitespace();
    let version = parse_http_version(parts.next().ok_or_else(|| "missing version".to_owned())?)?;
    let status = parts
        .next()
        .ok_or_else(|| "missing status".to_owned())?
        .parse::<u16>()
        .map_err(|_| "invalid status".to_owned())?;
    let mut builder = Response::builder()
        .version(version)
        .status(StatusCode::from_u16(status).map_err(|_| "invalid status".to_owned())?);
    let headers = builder
        .headers_mut()
        .ok_or_else(|| "response headers unavailable".to_owned())?;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| "malformed response header".to_owned())?;
        set_header(headers, name.trim(), value.trim());
    }
    builder.body(()).map_err(|error| error.to_string())
}

fn serialize_headers(headers: &HeaderMap) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (name, value) in headers {
        bytes.extend_from_slice(name.as_str().as_bytes());
        bytes.extend_from_slice(b": ");
        bytes.extend_from_slice(value.as_bytes());
        bytes.extend_from_slice(b"\r\n");
    }
    bytes.extend_from_slice(b"\r\n");
    bytes
}

fn parse_http_version(value: &str) -> Result<Version, String> {
    match value {
        "HTTP/1.0" => Ok(Version::HTTP_10),
        "HTTP/1.1" => Ok(Version::HTTP_11),
        _ => Err("invalid version".to_owned()),
    }
}

fn http_version_text(version: Version) -> &'static str {
    match version {
        Version::HTTP_10 => "HTTP/1.0",
        Version::HTTP_11 => "HTTP/1.1",
        _ => "HTTP/1.1",
    }
}

fn split_head(text: &str) -> Result<(&str, &str), String> {
    text.split_once("\r\n\r\n")
        .ok_or_else(|| "missing header terminator".to_owned())
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name)?.to_str().ok().map(ToOwned::to_owned)
}

fn set_header(headers: &mut HeaderMap, name: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    let name = HeaderName::from_str(name).expect("valid header name");
    let value = HeaderValue::from_str(value).expect("valid header value");
    headers.insert(name, value);
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => !(ip.is_private() || ip.is_loopback() || ip.is_unspecified()),
        IpAddr::V6(ip) => !(ip.is_loopback() || ip.is_unspecified()),
    }
}

fn is_unspecified_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_unspecified(),
        IpAddr::V6(ip) => ip.is_unspecified(),
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use basics::base_uint::Uint256;
    use basics::base64::base64_encode;
    use http::{HeaderMap, HeaderValue, StatusCode};
    use protocol::{KeyType, SecretKey, derive_public_key, sign_digest};

    use super::{
        EPOCH_OFFSET_SECONDS, FEATURE_COMPR, FEATURE_LEDGER_REPLAY, FEATURE_TXRR, FEATURE_VPRR,
        HandshakeContext, HandshakeVerificationContext, X_PROTOCOL_CTL, build_handshake,
        feature_enabled, get_feature_value, is_feature_value, make_features_request_header,
        make_features_response_header, make_request, make_response,
        make_shared_value_from_finished_messages, parse_http_request, parse_http_response,
        serialize_request, serialize_response, verify_handshake,
    };

    #[test]
    fn feature_helpers_match_cpp_handshake_test() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Protocol-Ctl",
            HeaderValue::from_static(
                "feature1=v1,v2,v3; feature2=v4; feature3=10; feature4=1; feature5=v6",
            ),
        );
        assert!(!feature_enabled(&headers, "feature1"));
        assert!(!is_feature_value(&headers, "feature1", "2"));
        assert!(is_feature_value(&headers, "feature1", "v1"));
        assert!(is_feature_value(&headers, "feature1", "v2"));
        assert!(is_feature_value(&headers, "feature1", "v3"));
        assert!(is_feature_value(&headers, "feature2", "v4"));
        assert!(!is_feature_value(&headers, "feature3", "1"));
        assert!(is_feature_value(&headers, "feature3", "10"));
        assert!(!is_feature_value(&headers, "feature4", "10"));
        assert!(is_feature_value(&headers, "feature4", "1"));
        assert!(!feature_enabled(&headers, "v6"));
        assert_eq!(
            get_feature_value(&headers, "feature5"),
            Some("v6".to_owned())
        );
    }

    #[test]
    fn request_and_response_round_trip() {
        let request = make_request(true, true, true, false, true);
        let wire = serialize_request(&request);
        let parsed = parse_http_request(&wire).expect("request should parse");
        assert_eq!(parsed.headers()["Crawl"], "public");

        let context = HandshakeContext::new(
            "n9MXXueo837zXLECxMtakUXs4QbQ5".to_owned(),
            "sig".to_owned(),
            7,
        );
        let response = make_response(
            true,
            &request,
            &context,
            crate::protocol_version::ProtocolVersion::new(2, 2),
            true,
            true,
            false,
            true,
        );
        let wire = serialize_response(&response);
        let parsed = parse_http_response(&wire).expect("response should parse");
        assert_eq!(parsed.status(), StatusCode::SWITCHING_PROTOCOLS);
    }

    #[test]
    fn verify_handshake_checks_signature_and_ip() {
        let secret = SecretKey::from_bytes([9u8; 32]);
        let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
        let shared_value = Uint256::from_slice(&[7u8; 32]).expect("uint256 width");
        let signature = sign_digest(&public, &secret, shared_value).expect("signature");
        let now_network_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
            - EPOCH_OFFSET_SECONDS;

        let mut headers = HeaderMap::new();
        let context = HandshakeContext {
            network_id: Some(21338),
            network_time: now_network_time,
            public_key: public.to_node_public_base58(),
            session_signature: base64_encode(&signature),
            instance_cookie: 4,
            server_domain: Some("example.com".to_owned()),
            remote_ip: Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10))),
            local_ip: Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 20))),
            closed_ledger: Some("A".repeat(64)),
            previous_ledger: Some("B".repeat(64)),
        };
        build_handshake(&mut headers, &context);

        let verify_context = HandshakeVerificationContext {
            shared_value,
            network_id: Some(21338),
            local_public_key: None,
            public_ip: Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10))),
            remote_ip: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 20)),
            clock_tolerance: Duration::from_secs(20),
        };
        let peer = verify_handshake(&headers, &verify_context).expect("handshake should verify");
        assert_eq!(peer.public_key, public);
    }

    #[test]
    fn build_handshake_ip_gating() {
        let mut headers = HeaderMap::new();
        let context = HandshakeContext {
            network_id: None,
            network_time: 1,
            public_key: "n9MXXueo837zXLECxMtakUXs4QbQ5".to_owned(),
            session_signature: "sig".to_owned(),
            instance_cookie: 7,
            server_domain: None,
            remote_ip: Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))),
            local_ip: Some(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            closed_ledger: None,
            previous_ledger: None,
        };
        build_handshake(&mut headers, &context);
        assert!(!headers.contains_key("Remote-IP"));
        assert!(!headers.contains_key("Local-IP"));
    }

    #[test]
    fn verify_handshake_rejects_ed25519_and_self_connection() {
        let shared_value = Uint256::from_slice(&[8u8; 32]).expect("uint256 width");
        let now_network_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
            - EPOCH_OFFSET_SECONDS;

        let ed_secret = SecretKey::from_bytes([11u8; 32]);
        let ed_public = derive_public_key(KeyType::Ed25519, &ed_secret).expect("public key");
        let mut headers = HeaderMap::new();
        let ed_context = HandshakeContext {
            network_id: None,
            network_time: now_network_time,
            public_key: ed_public.to_node_public_base58(),
            session_signature: base64_encode(b"sig"),
            instance_cookie: 1,
            server_domain: None,
            remote_ip: None,
            local_ip: None,
            closed_ledger: None,
            previous_ledger: None,
        };
        build_handshake(&mut headers, &ed_context);

        let verify_context = HandshakeVerificationContext {
            shared_value,
            network_id: None,
            local_public_key: None,
            public_ip: None,
            remote_ip: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 20)),
            clock_tolerance: Duration::from_secs(20),
        };
        assert_eq!(
            verify_handshake(&headers, &verify_context),
            Err("Unsupported public key type".to_owned())
        );

        let secp_secret = SecretKey::from_bytes([12u8; 32]);
        let secp_public =
            derive_public_key(KeyType::Secp256k1, &secp_secret).expect("secp public key");
        let secp_signature =
            sign_digest(&secp_public, &secp_secret, shared_value).expect("signature");
        let secp_context = HandshakeContext {
            public_key: secp_public.to_node_public_base58(),
            session_signature: base64_encode(&secp_signature),
            ..ed_context
        };
        headers.clear();
        build_handshake(&mut headers, &secp_context);
        let verify_context = HandshakeVerificationContext {
            local_public_key: Some(secp_public),
            ..verify_context
        };
        assert_eq!(
            verify_handshake(&headers, &verify_context),
            Err("Self connection".to_owned())
        );
    }

    #[test]
    fn make_shared_value_cookie_rules() {
        let shared = make_shared_value_from_finished_messages(&[1u8; 12], &[2u8; 12])
            .expect("finished messages should produce shared value");
        assert_ne!(shared, Uint256::zero());
        assert!(make_shared_value_from_finished_messages(&[1u8; 11], &[2u8; 12]).is_none());
        assert!(make_shared_value_from_finished_messages(&[3u8; 12], &[3u8; 12]).is_none());
    }

    #[test]
    fn feature_header_builders_match_cpp_shape() {
        assert_eq!(
            make_features_request_header(true, true, true, true),
            "compr=lz4;ledgerreplay=1;txrr=1;vprr=1;"
        );
        let request = make_request(true, true, true, true, true);
        assert_eq!(
            make_features_response_header(&request, true, true, false, true),
            format!("compr=lz4;{}=1;{}=1;", FEATURE_LEDGER_REPLAY, FEATURE_VPRR)
        );
        assert!(
            request.headers()[X_PROTOCOL_CTL]
                .to_str()
                .unwrap_or("")
                .contains(FEATURE_COMPR)
        );
        assert!(
            request.headers()[X_PROTOCOL_CTL]
                .to_str()
                .unwrap_or("")
                .contains(FEATURE_TXRR)
        );
    }
}
