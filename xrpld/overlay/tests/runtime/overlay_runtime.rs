use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::Once;
use std::time::{Duration, SystemTime};

use basics::base_uint::Uint256;
use http::Request;
use overlay::{
    Clock, ConnectAttemptError, ConnectionStep, Handoff, ManualClock, Overlay, OverlayHandoff,
    OverlayImpl, Peer, PeerImp, PeerSet, ProtocolPayload, Setup, SimplePeerSet, SlotState,
    TmProposeSet, TmTransaction,
};
use protocol::{JsonValue, KeyType, PublicKey, SecretKey, derive_public_key};
use rcgen::generate_simple_self_signed;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

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
    fn on_handoff(&self, _request: &Request<()>, _remote_address: SocketAddr) -> Handoff {
        Handoff::Accepted
    }
}

fn install_test_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

fn setup() -> Setup {
    install_test_crypto_provider();

    Setup {
        client_config: Some(Arc::new(
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoVerify))
                .with_no_client_auth(),
        )),
        tx_reduce_relay_min_peers: 1,
        tx_relay_percentage: 0,
        reduce_relay_wait: Duration::from_secs(0),
        vp_reduce_relay_max_selected_peers: 3,
        ..Default::default()
    }
}

fn server_config() -> Arc<rustls::ServerConfig> {
    install_test_crypto_provider();

    let certified_key =
        generate_simple_self_signed(vec!["localhost".to_owned()]).expect("self-signed certificate");
    let cert = CertificateDer::from(certified_key.cert.der().to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        certified_key.key_pair.serialize_der(),
    ));
    Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .expect("server tls config"),
    )
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
fn proposal_runtime_matches_slot_and_squelch_flow() {
    let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(Duration::from_secs(0)));
    let overlay = OverlayImpl::with_clock(setup(), Arc::new(TestHandoff), clock).expect("overlay");
    let validator = public_key(99);

    let peers = [peer(1, 11), peer(2, 12), peer(3, 13), peer(4, 14)];
    for peer in &peers {
        overlay.activate(Arc::clone(peer));
    }

    let proposal = TmProposeSet {
        propose_seq: 1,
        current_tx_hash: vec![1; 32],
        node_pub_key: validator.as_bytes().to_vec(),
        close_time: 2,
        signature: vec![3; 64],
        previousledger: vec![4; 32],
        added_transactions: Vec::new(),
        removed_transactions: Vec::new(),
        ..Default::default()
    };

    for uid in 1..=25 {
        overlay.relay_proposal(proposal.clone(), Uint256::from_u64(uid), validator);
    }

    assert_eq!(overlay.slot_state(validator), Some(SlotState::Selected));
    let squelch_messages = peers[3]
        .queued_messages()
        .into_iter()
        .filter(|message| matches!(message.protocol().payload, ProtocolPayload::Squelch(_)))
        .count();
    assert!(squelch_messages > 0);
}

#[test]
fn tx_reduce_relay_matches_disabled_send_and_enabled_queue_split() {
    let overlay = OverlayImpl::new(setup(), Arc::new(TestHandoff)).expect("overlay");
    let disabled = peer(1, 21);
    let enabled_a = peer(2, 22);
    let enabled_b = peer(3, 23);
    disabled.set_tx_reduce_relay_enabled(false);
    enabled_a.set_tx_reduce_relay_enabled(true);
    enabled_b.set_tx_reduce_relay_enabled(true);
    overlay.activate(Arc::clone(&disabled));
    overlay.activate(Arc::clone(&enabled_a));
    overlay.activate(Arc::clone(&enabled_b));

    overlay.relay_transaction(
        Uint256::from_u64(88),
        Some(TmTransaction {
            raw_transaction: vec![1, 2, 3],
            status: 1,
            receive_timestamp: None,
            deferred: None,
        }),
        &BTreeSet::new(),
    );

    assert_eq!(disabled.queued_messages().len(), 1);
    assert_eq!(enabled_a.queued_messages().len(), 1);
    assert!(enabled_b.queued_messages().is_empty());

    overlay.send_tx_queue();
    assert!(!enabled_b.queued_messages().is_empty());
}

#[test]
fn tx_metrics_surface_cpp_runtime_keys() {
    let overlay = OverlayImpl::new(setup(), Arc::new(TestHandoff)).expect("overlay");
    let peer = peer(7, 31);
    overlay.activate(peer);

    overlay.broadcast(&overlay::ProtocolMessage::new(
        ProtocolPayload::Transaction(TmTransaction {
            raw_transaction: vec![9; 64],
            status: 1,
            receive_timestamp: None,
            deferred: None,
        }),
    ));

    let JsonValue::Object(object) = overlay.tx_metrics() else {
        panic!("expected object");
    };
    assert!(object.contains_key("txr_tx_cnt"));
    assert!(object.contains_key("txr_selected_cnt"));
}

#[test]
fn activation_applies_runtime_membership_flags() {
    let overlay = OverlayImpl::new(setup(), Arc::new(TestHandoff)).expect("overlay");
    let reserved = peer(8, 41);
    let clustered = peer(9, 42);

    overlay
        .reservations()
        .insert_or_assign(overlay::PeerReservation {
            node_public: reserved.node_public(),
            description: "reserved".to_owned(),
        });
    assert!(
        overlay
            .cluster()
            .update(clustered.node_public(), "clustered", 0, SystemTime::now(),)
    );

    overlay.activate(Arc::clone(&reserved));
    overlay.activate(Arc::clone(&clustered));

    assert!(reserved.reserved());
    assert!(!reserved.cluster());
    assert!(clustered.cluster());
    assert!(!clustered.reserved());
}

#[test]
fn activation_enforces_peer_limit_but_allows_reserved_or_cluster_bypass() {
    let overlay = OverlayImpl::new(
        Setup {
            ip_limit: 32,
            peer_limit: 1,
            ..setup()
        },
        Arc::new(TestHandoff),
    )
    .expect("overlay");
    let first = peer(11, 44);
    let second = peer(12, 45);
    let reserved = peer(13, 46);
    let clustered = peer(14, 47);

    assert!(overlay.activate(Arc::clone(&first)));
    assert!(!overlay.activate(Arc::clone(&second)));

    overlay
        .reservations()
        .insert_or_assign(overlay::PeerReservation {
            node_public: reserved.node_public(),
            description: "reserved".to_owned(),
        });
    assert!(
        overlay
            .cluster()
            .update(clustered.node_public(), "clustered", 0, SystemTime::now(),)
    );

    assert!(overlay.activate(Arc::clone(&reserved)));
    assert!(overlay.activate(Arc::clone(&clustered)));
    assert_eq!(overlay.limit(), 1);
    assert_eq!(overlay.size(), 3);
    assert!(overlay.find_peer_by_short_id(second.id()).is_none());
}

#[test]
fn refresh_membership_state_evicts_excess_unreserved_peers_after_owner_source_change() {
    let overlay = OverlayImpl::new(
        Setup {
            peer_limit: 1,
            ..setup()
        },
        Arc::new(TestHandoff),
    )
    .expect("overlay");
    let external = Arc::new(overlay::PeerReservationTable::default());
    let first = peer(15, 48);
    let reserved = peer(16, 49);

    external.insert_or_assign(overlay::PeerReservation {
        node_public: reserved.node_public(),
        description: "wallet-backed".to_owned(),
    });
    overlay.set_peer_reservation_source(external.clone());

    assert!(overlay.activate(Arc::clone(&first)));
    assert!(overlay.activate(Arc::clone(&reserved)));
    assert_eq!(overlay.size(), 2);

    assert!(external.erase(reserved.node_public()).is_some());
    overlay.refresh_membership_state();

    assert_eq!(overlay.size(), 1);
    assert!(overlay.find_peer_by_short_id(first.id()).is_some());
    assert!(overlay.find_peer_by_short_id(reserved.id()).is_none());
}

#[test]
fn check_tracking_diverged_then_converged_range_behavior() {
    let overlay = OverlayImpl::new(setup(), Arc::new(TestHandoff)).expect("overlay");
    let tracked = peer(10, 43);
    tracked.record_ledger(Uint256::from_u64(10), 200);
    overlay.activate(Arc::clone(&tracked));

    assert!(tracked.has_range(200, 200));
    overlay.check_tracking(400);
    assert!(!tracked.has_range(200, 200));

    overlay.check_tracking(210);
    assert!(tracked.has_range(200, 200));
}

#[test]
fn peer_set_add_peers_score_order_and_duplicate_suppression() {
    let first: Arc<dyn Peer> = peer(1, 51);
    let second: Arc<dyn Peer> = peer(2, 52);
    let third: Arc<dyn Peer> = peer(3, 53);
    let peer_set = SimplePeerSet::new(vec![
        Arc::clone(&third),
        Arc::clone(&first),
        Arc::clone(&second),
    ]);

    let mut added = Vec::new();
    peer_set.add_peers(2, &mut |peer| peer.id() != 1, &mut |peer| {
        added.push(peer.id())
    });
    {
        let mut s = added.clone();
        s.sort();
        assert_eq!(s, vec![2, 3]);
    }

    let mut second_pass = Vec::new();
    peer_set.add_peers(3, &mut |_| true, &mut |peer| second_pass.push(peer.id()));
    assert_eq!(second_pass, vec![1]);
}

#[test]
fn deactivate_clears_relay_history_and_counts_disconnects() {
    let overlay = OverlayImpl::new(setup(), Arc::new(TestHandoff)).expect("overlay");
    let validator = public_key(77);
    let proposal = TmProposeSet {
        propose_seq: 1,
        current_tx_hash: vec![1; 32],
        node_pub_key: validator.as_bytes().to_vec(),
        close_time: 2,
        signature: vec![3; 64],
        previousledger: vec![4; 32],
        added_transactions: Vec::new(),
        removed_transactions: Vec::new(),
        ..Default::default()
    };
    let uid = Uint256::from_u64(99);
    let first_peer = peer(5, 61);

    overlay.activate(Arc::clone(&first_peer));
    assert!(
        overlay
            .relay_proposal(proposal.clone(), uid, validator)
            .is_empty()
    );
    assert_eq!(first_peer.queued_messages().len(), 1);

    overlay.on_peer_deactivate(first_peer.id());
    assert_eq!(overlay.peer_disconnect(), 1);
    assert_eq!(overlay.size(), 0);

    let second_peer = peer(5, 61);
    overlay.activate(Arc::clone(&second_peer));

    assert!(overlay.relay_proposal(proposal, uid, validator).is_empty());
    assert_eq!(second_peer.queued_messages().len(), 1);
}

#[tokio::test]
async fn listener_runtime_honors_overlay_stop_signal() {
    let overlay = OverlayImpl::new(
        Setup {
            server_config: Some(server_config()),
            ..setup()
        },
        Arc::new(TestHandoff),
    )
    .expect("overlay");

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("listener");
    let acceptor = overlay.bind(listener).expect("acceptor");
    let task = overlay.spawn_listener(acceptor);

    overlay.signal_stop();

    let result = timeout(Duration::from_secs(1), task)
        .await
        .expect("listener task must stop")
        .expect("listener join should succeed");
    assert!(result.is_ok());
    assert!(overlay.is_stopping());
    std::thread::spawn(move || drop(overlay))
        .join()
        .expect("overlay drop");
}

#[tokio::test]
async fn listener_runtime_honors_overlay_stop_signal_during_tls_handshake() {
    let overlay = OverlayImpl::new(
        Setup {
            server_config: Some(server_config()),
            ..setup()
        },
        Arc::new(TestHandoff),
    )
    .expect("overlay");

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("listener");
    let endpoint = listener.local_addr().expect("listener address");
    let acceptor = overlay.bind(listener).expect("acceptor");
    let task = overlay.spawn_listener(acceptor);

    let _client = TcpStream::connect(endpoint)
        .await
        .expect("client must connect");
    tokio::time::sleep(Duration::from_millis(50)).await;

    overlay.signal_stop();

    let result = timeout(Duration::from_secs(1), task)
        .await
        .expect("listener task must stop")
        .expect("listener join should succeed");
    assert!(result.is_ok());
    assert!(overlay.is_stopping());
    std::thread::spawn(move || drop(overlay))
        .join()
        .expect("overlay drop");
}

#[tokio::test]
async fn outbound_connect_runtime_honors_overlay_stop_signal() {
    let overlay = OverlayImpl::new(setup(), Arc::new(TestHandoff)).expect("overlay");

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("listener");
    let endpoint = listener.local_addr().expect("listener address");
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();

    let server = tokio::spawn(async move {
        let (_socket, _remote_address) = listener.accept().await.expect("accept");
        let _ = accepted_tx.send(());
        tokio::time::sleep(Duration::from_secs(5)).await;
    });

    let task = tokio::spawn(overlay.connect(endpoint));
    accepted_rx.await.expect("server must accept");
    overlay.signal_stop();

    let result = timeout(Duration::from_secs(1), task)
        .await
        .expect("connect task must stop")
        .expect("connect join should succeed");

    match result {
        Err(ConnectAttemptError::Timeout(ConnectionStep::ShutdownStarted)) => {}
        other => panic!("expected shutdown cancellation, got {other:?}"),
    }

    server.abort();
    assert!(overlay.is_stopping());
    std::thread::spawn(move || drop(overlay))
        .join()
        .expect("overlay drop");
}

#[tokio::test]
async fn overlay_peer_round_trip_activates_both_sides() {
    let listener_overlay = OverlayImpl::new(
        Setup {
            server_config: Some(server_config()),
            ..setup()
        },
        Arc::new(TestHandoff),
    )
    .expect("listener overlay");
    let client_overlay = OverlayImpl::new(setup(), Arc::new(TestHandoff)).expect("client overlay");

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("listener");
    let endpoint = listener.local_addr().expect("listener address");
    let acceptor = listener_overlay.bind(listener).expect("acceptor");
    let task = listener_overlay.spawn_listener(acceptor);

    let result = client_overlay
        .connect(endpoint)
        .await
        .expect("connect should succeed");
    assert_eq!(result.response.status().as_u16(), 101);

    timeout(Duration::from_secs(1), async {
        while listener_overlay.size() == 0 || client_overlay.size() == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("peer activation should complete");

    let client_peer = client_overlay
        .active_peers()
        .into_iter()
        .next()
        .expect("client peer should activate");
    let listener_peer = listener_overlay
        .active_peers()
        .into_iter()
        .next()
        .expect("listener peer should activate");

    assert_ne!(client_peer.node_public(), listener_peer.node_public());

    listener_overlay.signal_stop();
    let stop_result = timeout(Duration::from_secs(1), task)
        .await
        .expect("listener task must stop")
        .expect("listener join should succeed");
    assert!(stop_result.is_ok());
}

#[tokio::test]
async fn outbound_connect_reports_service_unavailable_when_listener_peer_limit_is_full() {
    let listener_overlay = OverlayImpl::new(
        Setup {
            server_config: Some(server_config()),
            peer_limit: 1,
            ..setup()
        },
        Arc::new(TestHandoff),
    )
    .expect("listener overlay");
    let client_overlay = OverlayImpl::new(setup(), Arc::new(TestHandoff)).expect("client overlay");

    assert!(listener_overlay.activate(peer(17, 50)));
    // Register a known redirect endpoint so the 503 response includes peer-ips
    listener_overlay.remember_redirect_endpoint(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 5017),
        1,
        std::time::SystemTime::now(),
    );

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("listener");
    let endpoint = listener.local_addr().expect("listener address");
    let acceptor = listener_overlay.bind(listener).expect("acceptor");
    let task = listener_overlay.spawn_listener(acceptor);

    let result = client_overlay.connect(endpoint).await;
    match result {
        Err(ConnectAttemptError::Redirect(peers)) => {
            assert_eq!(peers.len(), 1);
            assert_eq!(
                peers[0],
                SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 5017)
            );
        }
        other => panic!("expected redirect response, got {other:?}"),
    }
    assert_eq!(listener_overlay.size(), 1);

    listener_overlay.signal_stop();
    let stop_result = timeout(Duration::from_secs(1), task)
        .await
        .expect("listener task must stop")
        .expect("listener join should succeed");
    assert!(stop_result.is_ok());
    std::thread::spawn(move || drop(listener_overlay))
        .join()
        .expect("overlay drop");
    std::thread::spawn(move || drop(client_overlay))
        .join()
        .expect("client overlay drop");
}
