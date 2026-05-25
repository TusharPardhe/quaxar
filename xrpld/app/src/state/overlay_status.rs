//! Narrow overlay-backed status seam for `NetworkOPs::getServerInfo(...)`.
//!
//! The full reference status path reads a much larger live overlay owner. The Rust
//! status slice only needs a narrow read surface for peer/network/disconnect
//! counters, so this trait keeps that owner contract explicit without forcing
//! `ApplicationRoot` to pretend it already owns the broader overlay runtime.

use overlay::Overlay;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OverlayStatusSnapshot {
    pub peers: u64,
    pub network_id: Option<u32>,
    pub jq_trans_overflow: u64,
    pub peer_disconnects: u64,
    pub peer_disconnect_charges: u64,
}

impl OverlayStatusSnapshot {
    pub fn from_overlay<T>(overlay: &T) -> Self
    where
        T: Overlay + ?Sized,
    {
        Self {
            peers: overlay.size().min(u64::MAX as usize) as u64,
            network_id: overlay.network_id(),
            jq_trans_overflow: overlay.jq_trans_overflow(),
            peer_disconnects: overlay.peer_disconnect(),
            peer_disconnect_charges: overlay.peer_disconnect_charges(),
        }
    }
}

pub trait OverlayStatusSource: Send + Sync {
    fn status_snapshot(&self) -> OverlayStatusSnapshot;
}

impl<T> OverlayStatusSource for T
where
    T: Overlay + ?Sized,
{
    fn status_snapshot(&self) -> OverlayStatusSnapshot {
        OverlayStatusSnapshot::from_overlay(self)
    }
}

#[cfg(test)]
mod tests {
    use super::OverlayStatusSnapshot;
    use overlay::{Clock, ManualClock, Overlay, OverlayHandoff, OverlayImpl, PeerImp, Setup};
    use protocol::{KeyType, PublicKey, SecretKey, derive_public_key};
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;

    #[derive(Debug)]
    struct NoVerify;

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

    struct TestHandoff;

    impl OverlayHandoff for TestHandoff {
        fn on_handoff(
            &self,
            _request: &http::Request<()>,
            _remote_address: SocketAddr,
        ) -> overlay::Handoff {
            overlay::Handoff::Accepted
        }
    }

    fn setup(network_id: Option<u32>) -> Setup {
        static TLS_PROVIDER: OnceLock<()> = OnceLock::new();
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
            network_id,
            tx_reduce_relay_min_peers: 1,
            tx_relay_percentage: 0,
            reduce_relay_wait: Duration::from_secs(0),
            vp_reduce_relay_max_selected_peers: 3,
            ..Default::default()
        }
    }

    fn public_key(seed: u8) -> PublicKey {
        let secret = SecretKey::from_bytes([seed; 32]);
        derive_public_key(KeyType::Secp256k1, &secret).expect("public key")
    }

    fn peer(id: u32, seed: u8) -> Arc<PeerImp> {
        PeerImp::new(
            id,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5000 + id as u16),
            public_key(seed),
            format!("peer-{id}"),
        )
    }

    #[test]
    fn overlay_status_snapshot_reads_live_overlay_runtime() {
        let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(Duration::from_secs(0)));
        let overlay = OverlayImpl::with_clock(setup(Some(21_338)), Arc::new(TestHandoff), clock)
            .expect("overlay");
        let first = peer(1, 11);
        let second = peer(2, 12);

        overlay.activate(Arc::clone(&first));
        overlay.activate(Arc::clone(&second));
        overlay.inc_jq_trans_overflow();
        overlay.inc_jq_trans_overflow();
        overlay.inc_peer_disconnect();
        overlay.inc_peer_disconnect_charges();
        overlay.inc_peer_disconnect_charges();

        assert_eq!(
            OverlayStatusSnapshot::from_overlay(&overlay),
            OverlayStatusSnapshot {
                peers: 2,
                network_id: Some(21_338),
                jq_trans_overflow: 2,
                peer_disconnects: 1,
                peer_disconnect_charges: 2,
            }
        );
    }
}
