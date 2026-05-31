//! Tests for server state.

#[path = "../server_info_owned_source/server_info_state_support.rs"]
mod server_info_state_support;

pub(super) use std::{
    cell::RefCell,
    collections::BTreeMap,
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) use app::{
    ApplicationRoot, NetworkOpsOperatingMode, PublishedGrpcPort, ServerPortSetup, ServerPortsSetup,
    StatusRpcGitInfo, StatusRpcLastClose, UnsupportedMajorityWarningDetails,
};
pub(super) use ledger::{Fees, Ledger, LedgerHeader};
pub(super) use overlay::{Overlay, OverlayHandoff, OverlayImpl, PeerImp, Setup};
pub(super) use perflog::{JobType, PerfLog};
pub(super) use protocol::JsonValue;
pub(super) use protocol::{KeyType, PublicKey, SecretKey, derive_public_key, get_version_string};
pub(super) use rpc::{
    ApplicationServerInfo, JsonContext, JsonContextHeaders, RpcRole, ServerStateSource,
    do_server_state,
};
pub(super) use rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
pub(super) use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
pub(super) use serde_json::json;
use server_info_state_support::{TestPerfLogReportSource, make_test_perf_log};
pub(super) use std::net::{IpAddr, Ipv4Addr, SocketAddr};

#[derive(Debug)]
pub(super) struct NoVerify;

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

pub(super) struct TestHandoff;

impl OverlayHandoff for TestHandoff {
    fn on_handoff(
        &self,
        _request: &http::Request<()>,
        _remote_address: SocketAddr,
    ) -> overlay::Handoff {
        overlay::Handoff::Accepted
    }
}

pub(super) fn overlay_setup(network_id: Option<u32>) -> Setup {
    static TLS_PROVIDER: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    TLS_PROVIDER.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    Setup {
        client_config: Some(Arc::new(
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoVerify))
                .with_no_client_auth(),
        )),
        server_config: None,
        fixed_peer_ips: HashSet::new(),
        public_ip: None,
        ip_limit: 0,
        peer_limit: 0,
        verify_endpoints: true,
        crawl_options: 0,
        network_id,
        vl_enabled: true,
        tx_reduce_relay_enabled: true,
        tx_reduce_relay_min_peers: 1,
        tx_relay_percentage: 0,
        vp_reduce_relay_base_squelch_enabled: true,
        vp_reduce_relay_max_selected_peers: 3,
        reduce_relay_wait: Duration::from_secs(0),
    }
}

pub(super) fn overlay_public_key(seed: u8) -> PublicKey {
    let secret = SecretKey::from_bytes([seed; 32]);
    derive_public_key(KeyType::Secp256k1, &secret).expect("public key")
}

pub(super) fn overlay_peer(id: u32, seed: u8) -> Arc<PeerImp> {
    PeerImp::new(
        id,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5000 + id as u16),
        overlay_public_key(seed),
        format!("peer-{id}"),
    )
}

#[derive(Debug, Default)]
pub(super) struct FakeServerStateSource {
    calls: RefCell<Vec<(bool, bool, bool)>>,
}

impl ServerStateSource for FakeServerStateSource {
    fn get_server_info(&self, human: bool, admin: bool, counters: bool) -> JsonValue {
        self.calls.borrow_mut().push((human, admin, counters));
        JsonValue::Object(BTreeMap::from([
            ("human".to_owned(), JsonValue::Bool(human)),
            ("admin".to_owned(), JsonValue::Bool(admin)),
            ("counters".to_owned(), JsonValue::Bool(counters)),
        ]))
    }
}

pub(super) fn context<'a, Env>(
    params: &'a JsonValue,
    env: &'a Env,
    role: RpcRole,
) -> JsonContext<'a, Env> {
    JsonContext {
        params,
        env,
        role,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: false,
    }
}

pub(super) fn sample_ledger(seq: u32, close_time: u32, hash_byte: u8) -> Arc<Ledger> {
    let mut ledger = Ledger::from_ledger_seq_and_close_time(seq, close_time, false);
    ledger.set_ledger_info(LedgerHeader {
        hash: basics::sha_map_hash::SHAMapHash::new(basics::base_uint::Uint256::from_array(
            [hash_byte; 32],
        )),
        ..ledger.header()
    });
    ledger.set_fees(Fees {
        base: 10,
        reserve: 2_000_000,
        increment: 200_000,
    });
    Arc::new(ledger)
}

mod admin_fields;
mod response_shape;
