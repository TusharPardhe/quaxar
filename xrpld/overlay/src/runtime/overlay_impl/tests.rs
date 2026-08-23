use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Once;
use std::sync::RwLock;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use basics::base_uint::Uint256;
use http::{HeaderMap, Request, Response};
use protocol::{
    AccountID, JsonValue, KeyType, PublicKey, STAmount, STTx, STValidation, SecretKey,
    VF_FULL_VALIDATION, calc_node_id, derive_public_key, get_field_by_symbol,
};
use resource::{Charge, Disposition};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::time::timeout;

use super::{
    OverlayAcceptor, OverlayHandoff, OverlayImpl, OverlayInboundRouter, PEERFINDER_LIVE_CACHE_TTL,
    PEERFINDER_MAX_ACCEPTED_ENDPOINTS, PEERFINDER_MAX_HOPS, PEERFINDER_REDIRECT_ENDPOINT_COUNT,
    PeerReservation, PeerReservationSource, PeerReservationTable, RelayKind,
    is_valid_peer_endpoint, read_http_request, validated_ledger_is_recent_for_peer_tracking,
};
use crate::message::{
    Message, ProtocolMessage, ProtocolPayload, TmEndpoints, TmGetLedger, TmGetObjectByHash,
    TmHaveTransactionSet, TmHaveTransactions, TmLedgerData, TmManifests, TmPing,
    TmProofPathRequest, TmProofPathResponse, TmProposeSet, TmReplayDeltaRequest,
    TmReplayDeltaResponse, TmSquelch, TmStatusChange, TmTransaction, TmTransactions, TmValidation,
    TmValidatorList, TmValidatorListCollection, decode_protocol_message, wire,
};
use crate::overlay::Overlay;
use crate::overlay::{Handoff, Setup};
use crate::peer::{Peer, ProtocolFeature};
use crate::peer_imp::{PeerImp, Tracking};
use crate::router::MessageRouter;
use crate::session::PeerSessionStarter;
use crate::slot::{Clock, ManualClock, SlotState};
use crate::traffic_count::TrafficCategory;
use crate::{Cluster, ConnectAttemptError, ConnectAttemptResult};

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

#[derive(Default)]
struct ExternalReservationSource {
    reservations: RwLock<BTreeMap<PublicKey, String>>,
}

impl ExternalReservationSource {
    fn insert(&self, node_public: PublicKey, description: &str) {
        self.reservations
            .write()
            .expect("external reservation source lock")
            .insert(node_public, description.to_owned());
    }

    fn erase(&self, node_public: PublicKey) {
        self.reservations
            .write()
            .expect("external reservation source lock")
            .remove(&node_public);
    }
}

impl PeerReservationSource for ExternalReservationSource {
    fn contains(&self, node_public: PublicKey) -> bool {
        self.reservations
            .read()
            .expect("external reservation source lock")
            .contains_key(&node_public)
    }
}

fn validator(seed: u8) -> PublicKey {
    let secret = SecretKey::from_bytes([seed; 32]);
    derive_public_key(KeyType::Secp256k1, &secret).expect("validator key")
}

fn install_test_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

fn peer(id: u32, seed: u8) -> Arc<PeerImp> {
    PeerImp::new(
        id,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51235 + id as u16),
        validator(seed),
        format!("peer-{id}"),
    )
}

fn inbound_peer(id: u32, seed: u8) -> Arc<PeerImp> {
    PeerImp::new_with_inbound(
        id,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51235 + id as u16),
        true,
        validator(seed),
        format!("inbound-peer-{id}"),
    )
}

fn directional_default_setup() -> Setup {
    Setup {
        peer_limit: 21,
        want_incoming: true,
        ..test_setup()
    }
}

#[test]
fn default_rippled_budget_inbound_exhaustion_does_not_block_outbound() {
    let overlay =
        OverlayImpl::new(directional_default_setup(), Arc::new(TestHandoff)).expect("overlay");
    assert_eq!(
        overlay.peer_limits(),
        crate::overlay::PeerLimits {
            max_peers: 21,
            inbound_max: 11,
            outbound_max: 10,
        }
    );

    let inbound = (0..11)
        .map(|index| inbound_peer(100 + index, 100 + index as u8))
        .collect::<Vec<_>>();
    for peer in &inbound {
        assert!(overlay.activate(Arc::clone(peer)));
    }
    assert!(!overlay.activate(inbound_peer(111, 111)));
    assert_eq!(overlay.active_inbound_peers_count(), 11);
    assert_eq!(overlay.active_outbound_peers_count(), 0);

    // `Counts::canActivate` compares only outActive/outMax for an outbound
    // slot, so an exhausted inbound budget cannot block this peer.
    assert!(overlay.activate(peer(200, 200)));
    assert_eq!(overlay.active_outbound_peers_count(), 1);

    // Deactivation removes the direction-specific slot and immediately frees
    // one inbound admission, matching Counts::remove(Active).
    overlay.on_peer_deactivate(inbound[0].id());
    assert_eq!(overlay.active_inbound_peers_count(), 10);
    assert!(overlay.activate(inbound_peer(112, 112)));
    assert_eq!(overlay.active_inbound_peers_count(), 11);
}

#[test]
fn default_rippled_budget_outbound_exhaustion_does_not_block_inbound() {
    let overlay =
        OverlayImpl::new(directional_default_setup(), Arc::new(TestHandoff)).expect("overlay");
    assert_eq!(overlay.peer_limits().outbound_max, 10);
    assert_eq!(overlay.peer_limits().inbound_max, 11);

    for index in 0..10 {
        assert!(overlay.activate(peer(220 + index, 120 + index as u8)));
    }
    assert!(!overlay.activate(peer(230, 130)));
    assert_eq!(overlay.active_outbound_peers_count(), 10);
    assert_eq!(overlay.active_inbound_peers_count(), 0);

    // `Counts::canActivate` compares only inActive/inMax for an inbound slot.
    assert!(overlay.activate(inbound_peer(240, 140)));
    assert_eq!(overlay.active_inbound_peers_count(), 1);
}

/// Drop an overlay safely from within an async test context.
/// Tokio panics if a runtime is dropped inside another runtime,
/// so we move the drop to a dedicated std thread.
fn drop_overlay_safely(overlay: Arc<OverlayImpl>) {
    std::thread::spawn(move || drop(overlay))
        .join()
        .expect("overlay drop thread");
}

fn test_setup() -> Setup {
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
        verify_endpoints: false,
        vp_reduce_relay_max_selected_peers: 3,
        reduce_relay_wait: Duration::from_secs(0),
        ..Default::default()
    }
}

fn account(hex: &str) -> AccountID {
    AccountID::from_hex(hex).expect("account hex")
}

fn payment_tx(sequence: u32) -> STTx {
    let source = account("1111111111111111111111111111111111111111");
    let destination = account("2222222222222222222222222222222222222222");

    STTx::new(protocol::TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), source);
        tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    })
}

fn tx_message(sequence: u32) -> TmTransaction {
    let tx = payment_tx(sequence);
    TmTransaction {
        raw_transaction: tx.get_serializer().data().to_vec(),
        status: 1,
        receive_timestamp: None,
        deferred: None,
    }
}

fn validation_message(seed: u8, sign_time: u32, ledger_fill: u8) -> TmValidation {
    let secret = SecretKey::from_bytes([seed; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("validation public");
    let node_id = calc_node_id(&public);
    let validation = STValidation::new_signed(sign_time, &public, node_id, &secret, |validation| {
        validation.set_field_h256(
            get_field_by_symbol("sfLedgerHash"),
            Uint256::from_array([ledger_fill; 32]),
        );
        validation.set_field_h256(
            get_field_by_symbol("sfConsensusHash"),
            Uint256::from_array([ledger_fill.wrapping_add(1); 32]),
        );
        validation.set_field_u32(get_field_by_symbol("sfLedgerSequence"), 55);
        validation.set_flag(VF_FULL_VALIDATION);
    })
    .expect("validation");

    #[allow(deprecated)]
    TmValidation {
        validation: validation.get_serialized(),
        checked_signature: None,
        hops: None,
    }
}

#[test]
fn reservation_table_replaces_and_lists_in_key_order() {
    let table = PeerReservationTable::default();
    let alpha = validator(1);
    let beta = validator(2);

    assert!(
        table
            .insert_or_assign(PeerReservation {
                node_public: beta,
                description: "beta".to_owned(),
            })
            .is_none()
    );
    assert!(
        table
            .insert_or_assign(PeerReservation {
                node_public: alpha,
                description: "alpha".to_owned(),
            })
            .is_none()
    );
    let previous = table
        .insert_or_assign(PeerReservation {
            node_public: alpha,
            description: "alpha-2".to_owned(),
        })
        .expect("previous reservation");
    assert_eq!(previous.description, "alpha");
    assert_eq!(table.list().len(), 2);
    assert!(table.contains(beta));
    assert_eq!(table.erase(beta).expect("removed").description, "beta");
}

#[test]
fn proposal_relay_updates_slot_and_sends_squelch_control() {
    let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(Duration::from_secs(0)));
    let overlay = OverlayImpl::with_clock(test_setup(), Arc::new(TestHandoff), clock.clone())
        .expect("overlay");

    let a = peer(1, 11);
    let b = peer(2, 12);
    let c = peer(3, 13);
    let d = peer(4, 14);
    overlay.activate(a.clone());
    overlay.activate(b.clone());
    overlay.activate(c.clone());
    overlay.activate(d.clone());

    let validator = validator(99);
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

    let sources = [&a, &b, &c, &d];
    for uid in 1..=88 {
        let source = sources[(uid as usize - 1) % sources.len()];
        assert!(overlay.register_relay_source(
            RelayKind::Proposal,
            Uint256::from_u64(uid),
            source.id(),
        ));
        overlay.relay_proposal(proposal.clone(), Uint256::from_u64(uid), validator);
    }

    assert_eq!(overlay.slot_state(validator), Some(SlotState::Selected));
    let squelch_messages = [&a, &b, &c, &d]
        .into_iter()
        .flat_map(|peer| peer.queued_messages())
        .filter(|message| matches!(message.protocol().payload, ProtocolPayload::Squelch(_)))
        .count();
    assert!(squelch_messages > 0);
}

#[test]
fn recent_duplicate_ingress_updates_source_peer_slot() {
    let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(Duration::from_secs(0)));
    let overlay =
        OverlayImpl::with_clock(test_setup(), Arc::new(TestHandoff), clock).expect("overlay");
    let first = peer(1, 11);
    let duplicate = peer(2, 12);
    overlay.activate(first.clone());
    overlay.activate(duplicate.clone());

    let validator = validator(99);
    let uid = Uint256::from_u64(99);
    let proposal = TmProposeSet {
        propose_seq: 1,
        current_tx_hash: vec![1; 32],
        node_pub_key: validator.as_bytes().to_vec(),
        close_time: 2,
        signature: vec![3; 64],
        previousledger: vec![4; 32],
        ..Default::default()
    };

    assert!(overlay.register_relay_source(RelayKind::Proposal, uid, first.id()));
    overlay.relay_proposal(proposal, uid, validator);
    assert!(!overlay.register_relay_source(RelayKind::Proposal, uid, duplicate.id()));
    overlay.update_slot_for_recent_duplicate(
        RelayKind::Proposal,
        uid,
        validator,
        duplicate.id(),
        crate::message::ProtocolMessageType::MtProposeLedger,
    );

    assert!(overlay.slot_peers(validator).contains_key(&duplicate.id()));
}

#[test]
fn local_validation_suppression_deduplicates_echo_without_marking_relayed() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    let source = peer(1, 31);
    let recipient = peer(2, 32);
    overlay.activate(source.clone());
    overlay.activate(recipient.clone());

    let uid = Uint256::from_u64(101);
    let signer = validator(100);
    overlay.suppress_validation(uid);
    assert!(!overlay.admit_validation_source(uid, signer, source.id()));

    // A suppression-only entry has no relay timestamp: the local broadcast
    // stays independent, while a later echo remains excluded as its source.
    let relayed = overlay.relay_validation(TmValidation::default(), uid, signer);
    assert_eq!(relayed, BTreeSet::from([source.id()]));
    assert!(source.queued_messages().is_empty());
    assert_eq!(recipient.queued_messages().len(), 1);
}

#[test]
fn tx_reduce_relay_selects_enabled_peers_and_queues_the_rest() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");

    let disabled = peer(1, 21);
    let enabled_a = peer(2, 22);
    let enabled_b = peer(3, 23);
    disabled.set_tx_reduce_relay_enabled(false);
    enabled_a.set_tx_reduce_relay_enabled(true);
    enabled_b.set_tx_reduce_relay_enabled(true);
    overlay.activate(disabled.clone());
    overlay.activate(enabled_a.clone());
    overlay.activate(enabled_b.clone());

    overlay.relay_transaction(
        Uint256::from_u64(88),
        Some(crate::TmTransaction {
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
    let JsonValue::Object(metrics) = overlay.tx_metrics() else {
        panic!("tx metrics json");
    };
    assert!(metrics.contains_key("txr_selected_cnt"));
}

#[test]
fn activate_applies_cluster_and_reservation_membership() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    let reserved = peer(10, 41);
    let clustered = peer(11, 42);

    overlay.reservations().insert_or_assign(PeerReservation {
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
fn activate_can_use_external_peer_reservation_source_and_refresh_membership() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    let reserved = peer(20, 51);
    let external = Arc::new(ExternalReservationSource::default());

    external.insert(reserved.node_public(), "wallet-backed");
    overlay.set_peer_reservation_source(external.clone());
    overlay.activate(Arc::clone(&reserved));

    assert!(overlay.reservations().list().is_empty());
    assert!(reserved.reserved());

    external.erase(reserved.node_public());
    overlay.refresh_membership_state();
    assert!(!reserved.reserved());

    external.insert(reserved.node_public(), "wallet-backed-again");
    overlay.refresh_membership_state();
    assert!(reserved.reserved());
}

#[test]
fn activate_can_use_external_cluster_source_and_refresh_membership() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    let clustered = peer(21, 52);
    let external = Arc::new(Cluster::new());

    overlay.set_cluster_source(Arc::clone(&external));
    overlay.activate(Arc::clone(&clustered));
    assert!(!clustered.cluster());

    assert!(external.update(
        clustered.node_public(),
        "wallet-cluster",
        0,
        SystemTime::now(),
    ));
    overlay.refresh_membership_state();
    assert!(clustered.cluster());
    assert_eq!(
        overlay.cluster().member(clustered.node_public()),
        Some("wallet-cluster".to_owned())
    );
}

#[test]
fn activate_enforces_peer_limit_for_unreserved_peers_only() {
    let overlay = OverlayImpl::new(
        Setup {
            ip_limit: 99,
            peer_limit: 1,
            peer_limit_in: Some(0),
            peer_limit_out: Some(1),
            ..test_setup()
        },
        Arc::new(TestHandoff),
    )
    .expect("overlay");
    let first = peer(22, 53);
    let second = peer(23, 54);
    let reserved = peer(24, 55);
    let clustered = peer(25, 56);

    assert!(overlay.activate(Arc::clone(&first)));
    assert!(!overlay.activate(Arc::clone(&second)));

    overlay.reservations().insert_or_assign(PeerReservation {
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
    // rippled: config.maxPeers = 0 when explicit in/out limits are set
    assert_eq!(overlay.limit(), 0);
    assert_eq!(overlay.size(), 3);
    assert!(reserved.reserved());
    assert!(clustered.cluster());
    assert!(overlay.find_peer_by_short_id(second.id()).is_none());
}

#[test]
fn active_outbound_peer_count_excludes_inbound_fixed_and_reserved_peers() {
    let overlay = OverlayImpl::new(
        Setup {
            fixed_peer_ips: HashSet::from([IpAddr::V4(Ipv4Addr::new(127, 0, 0, 99))]),
            ..test_setup()
        },
        Arc::new(TestHandoff),
    )
    .expect("overlay");
    let outbound = peer(29, 60);
    let inbound = PeerImp::new_with_inbound(
        30,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51265),
        true,
        validator(61),
        "peer-30",
    );
    let fixed = PeerImp::new(
        31,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 99)), 51266),
        validator(62),
        "peer-31",
    );
    let reserved = peer(32, 63);

    overlay.reservations().insert_or_assign(PeerReservation {
        node_public: reserved.node_public(),
        description: "reserved".to_owned(),
    });

    assert!(overlay.activate(Arc::clone(&outbound)));
    assert!(overlay.activate(Arc::clone(&inbound)));
    assert!(overlay.activate(Arc::clone(&fixed)));
    assert!(overlay.activate(Arc::clone(&reserved)));
    assert!(fixed.fixed());
    assert!(reserved.reserved());
    assert_eq!(overlay.size(), 4);
    assert_eq!(overlay.active_outbound_peers_count(), 1);
}

#[test]
fn activate_enforces_peer_limit_for_fixed_peers_slots() {
    let overlay = OverlayImpl::new(
        Setup {
            peer_limit: 1,
            peer_limit_in: Some(0),
            peer_limit_out: Some(1),
            fixed_peer_ips: HashSet::from([IpAddr::V4(Ipv4Addr::new(127, 0, 0, 99))]),
            ..test_setup()
        },
        Arc::new(TestHandoff),
    )
    .expect("overlay");
    let counted = peer(33, 64);
    let fixed = PeerImp::new(
        34,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 99)), 51267),
        validator(65),
        "peer-34",
    );

    assert!(overlay.activate(Arc::clone(&counted)));
    assert!(overlay.activate(Arc::clone(&fixed)));
    assert!(fixed.fixed());
    // rippled: config.maxPeers = 0 when explicit in/out limits are set
    assert_eq!(overlay.limit(), 0);
    assert_eq!(overlay.size(), 2);
    assert_eq!(overlay.active_outbound_peers_count(), 1);
}

#[test]
fn inbound_blacklisted_resource_consumer_is_rejected_before_peer_slot_and_released() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 51235);

    let blacklisted = overlay.resource_manager.new_inbound_endpoint(address);
    assert_eq!(
        blacklisted.charge(Charge::new(800_001, "blacklist test")),
        Disposition::Drop
    );
    assert!(blacklisted.disconnect_with_manager_journal());
    drop(blacklisted);

    assert!(
        overlay.reserve_inbound(address).is_none(),
        "a dropped Resource consumer must reject before peer admission"
    );
    assert!(
        overlay
            .inbound_reservations
            .lock()
            .expect("inbound reservation lock")
            .by_ip
            .is_empty(),
        "the rejected reservation must not retain an inbound IP slot"
    );
    assert!(
        overlay.resource_manager.on_write()["inbound"]
            .as_array()
            .expect("inbound resource list")
            .is_empty(),
        "the temporary dropped consumer must be released"
    );
}

#[test]
fn inbound_reservation_counts_pending_peer_admissions_and_transfers_on_activation() {
    let overlay = OverlayImpl::new(
        Setup {
            ip_limit: 1,
            ..test_setup()
        },
        Arc::new(TestHandoff),
    )
    .expect("overlay");
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 51235);

    let reservation = overlay.reserve_inbound(address).expect("first reservation");
    assert!(
        overlay.reserve_inbound(address).is_none(),
        "a pending peer handoff must consume its inbound IP slot"
    );
    drop(reservation);
    let reservation = overlay
        .reserve_inbound(address)
        .expect("released reservation");

    let active = PeerImp::new_with_inbound(91, address, true, validator(91), "inbound-91");
    assert!(overlay.activate_with_inbound_reservation(Arc::clone(&active), reservation));
    assert!(
        overlay.reserve_inbound(address).is_none(),
        "activation retains the reservation as the active peer's IP resource"
    );
    overlay.on_peer_deactivate(active.id());
    assert!(overlay.reserve_inbound(address).is_some());
}

#[test]
fn inbound_ip_limit_is_applied_only_to_public_remote_addresses() {
    let overlay = OverlayImpl::new(
        Setup {
            ip_limit: 1,
            ..test_setup()
        },
        Arc::new(TestHandoff),
    )
    .expect("overlay");
    let private_address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51235);

    let first = overlay
        .reserve_inbound(private_address)
        .expect("first private reservation");
    let second = overlay
        .reserve_inbound(private_address)
        .expect("second private reservation");
    assert!(
        overlay
            .inbound_reservations
            .lock()
            .expect("inbound reservation lock")
            .by_ip
            .is_empty(),
        "private addresses must not consume PeerFinder ip_limit slots"
    );
    drop(first);
    drop(second);
}

#[test]
fn connect_as_peer_token_matches_rippled_comma_list_semantics() {
    let peer = Request::builder()
        .header("Connect-As", "client, PeEr")
        .body(())
        .expect("peer request");
    let non_peer = Request::builder()
        .header("Connect-As", "client, public")
        .body(())
        .expect("non-peer request");

    assert!(super::request_connects_as_peer(&peer));
    assert!(!super::request_connects_as_peer(&non_peer));
}

#[test]
fn transaction_get_objects_query_precedes_generic_ledger_hash_validation() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    let peer = peer(97, 97);
    peer.set_tx_reduce_relay_enabled(true);
    let mut router = OverlayInboundRouter {
        overlay: &overlay,
        peer: &peer,
        ledger_data_deferred: false,
    };
    let request = TmGetObjectByHash {
        r#type: wire::tm_get_object_by_hash::ObjectType::OtTransactions as i32,
        query: true,
        // Transaction queries are dispatched before generic ledger-hash
        // validation in rippled, so this irrelevant malformed selector must
        // not prevent queueing the request.
        ledger_hash: Some(vec![0; 31]),
        fat: None,
        objects: Vec::new(),
    };

    let _ = router.on_get_objects(&request);

    assert_eq!(overlay.queued_inbound_snapshot().get_objects.len(), 1);
    assert!(peer.charges().is_empty());
}

#[test]
fn disabled_transaction_get_objects_query_is_charged_as_malformed_request() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    let peer = peer(98, 98);
    peer.set_tx_reduce_relay_enabled(false);
    let mut router = OverlayInboundRouter {
        overlay: &overlay,
        peer: &peer,
        ledger_data_deferred: false,
    };
    let request = TmGetObjectByHash {
        r#type: wire::tm_get_object_by_hash::ObjectType::OtTransactions as i32,
        query: true,
        ledger_hash: Some(vec![0; 31]),
        fat: None,
        objects: Vec::new(),
    };

    let _ = router.on_get_objects(&request);

    assert!(overlay.queued_inbound_snapshot().get_objects.is_empty());
    assert_eq!(peer.charges().len(), 1);
    let (charge, context) = &peer.charges()[0];
    assert_eq!(charge, &*resource::FEE_MALFORMED_REQUEST);
    assert_eq!(context, "tx reduce-relay disabled");
}

#[test]
fn inbound_feature_advertisement_honors_runtime_feature_switches() {
    let overlay = OverlayImpl::new(
        Setup {
            tx_reduce_relay_enabled: false,
            vp_reduce_relay_base_squelch_enabled: false,
            ..test_setup()
        },
        Arc::new(TestHandoff),
    )
    .expect("overlay");
    let request = crate::handshake::make_request(true, true, true, true, true);
    let (compr, replay, txrr, vprr) = overlay.inbound_handshake_features();
    let response = crate::handshake::make_response(
        true,
        &request,
        &overlay.handshake_context(),
        crate::ProtocolVersion::new(2, 2),
        compr,
        replay,
        txrr,
        vprr,
    );
    let features = response.headers()["X-Protocol-Ctl"]
        .to_str()
        .expect("feature header");

    assert!(features.contains("compr=lz4"));
    assert!(features.contains("ledgerreplay=1"));
    assert!(!features.contains("txrr=1"));
    assert!(!features.contains("vprr=1"));

    let requested_peer = peer(96, 96);
    overlay.configure_connected_peer(&requested_peer, request.headers());
    assert!(
        !requested_peer.tx_reduce_relay_enabled(),
        "a peer request must not enable a locally disabled tx relay feature"
    );
}

#[tokio::test]
async fn listener_accepts_next_socket_while_earlier_tls_handshake_is_pending() {
    let overlay = Arc::new(OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay"));
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("listener");
    let endpoint = listener.local_addr().expect("listener endpoint");
    let acceptor = OverlayAcceptor {
        listener: Arc::new(listener),
        acceptor: Arc::new(
            openssl::ssl::SslAcceptor::mozilla_intermediate(openssl::ssl::SslMethod::tls())
                .expect("ssl acceptor")
                .build(),
        ),
    };

    let first_once = {
        let overlay = Arc::clone(&overlay);
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            overlay
                .run_listener_once(&acceptor, overlay.stop_receiver())
                .await
        })
    };
    let first_client = tokio::net::TcpStream::connect(endpoint)
        .await
        .expect("first client");
    timeout(Duration::from_secs(1), first_once)
        .await
        .expect("first accept must not wait for TLS")
        .expect("first accept join")
        .expect("first accept result");

    let second_once = {
        let overlay = Arc::clone(&overlay);
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            overlay
                .run_listener_once(&acceptor, overlay.stop_receiver())
                .await
        })
    };
    let second_client = tokio::net::TcpStream::connect(endpoint)
        .await
        .expect("second client");
    timeout(Duration::from_secs(1), second_once)
        .await
        .expect("second accept must not be blocked by first TLS handshake")
        .expect("second accept join")
        .expect("second accept result");

    assert_eq!(
        *overlay
            .session_tasks
            .active
            .lock()
            .expect("overlay session task lock"),
        2,
        "both unfinished TLS handshakes must remain independently tracked"
    );

    overlay.signal_stop();
    timeout(Duration::from_secs(1), async {
        loop {
            if *overlay
                .session_tasks
                .active
                .lock()
                .expect("overlay session task lock")
                == 0
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("stop must cancel tracked inbound handshakes");

    drop(first_client);
    drop(second_client);
    drop_overlay_safely(overlay);
}

#[test]
fn activation_rejects_duplicate_node_key_and_deactivation_keeps_the_owner_mapping() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    let first = peer(92, 92);
    let duplicate = PeerImp::new(
        93,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51328),
        first.node_public(),
        "duplicate-node-key",
    );

    assert!(overlay.activate(Arc::clone(&first)));
    assert!(!overlay.activate(Arc::clone(&duplicate)));
    assert_eq!(overlay.size(), 1);
    assert_eq!(
        overlay
            .find_peer_by_public_key(first.node_public())
            .expect("first key mapping")
            .id(),
        first.id()
    );

    overlay.on_peer_deactivate(duplicate.id());
    assert_eq!(
        overlay
            .find_peer_by_public_key(first.node_public())
            .expect("duplicate close must not erase owner mapping")
            .id(),
        first.id()
    );
    overlay.on_peer_deactivate(first.id());
    assert!(
        overlay
            .find_peer_by_public_key(first.node_public())
            .is_none()
    );
}

#[test]
fn outbound_endpoint_activity_covers_active_and_pending_connections() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7)), 51235);

    assert!(!overlay.outbound_endpoint_is_active_or_pending(target));
    let reservation = overlay
        .reserve_outbound_attempt(target)
        .expect("attempt reservation");
    assert!(overlay.outbound_endpoint_is_active_or_pending(target));
    drop(reservation);

    let active = PeerImp::new(94, target, validator(94), "outbound-94");
    assert!(overlay.activate(active));
    assert!(overlay.outbound_endpoint_is_active_or_pending(target));
}

#[tokio::test]
async fn duplicate_outbound_attempt_is_suppression_not_protocol_failure() {
    let overlay = Arc::new(OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay"));
    let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 8)), 51235);

    let _reservation = overlay
        .reserve_outbound_attempt(target)
        .expect("attempt reservation");
    assert!(matches!(
        overlay.connect(target).await,
        Err(ConnectAttemptError::DuplicateOutboundAttempt)
    ));
    drop_overlay_safely(overlay);
}

#[tokio::test]
async fn healthy_active_fixed_peer_suppresses_duplicate_without_failure_backoff() {
    let overlay = Arc::new(OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay"));
    let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 9)), 51235);
    overlay.remember_fixed_peer_endpoint(target);
    let failures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    overlay.set_outbound_peer_failure_handler({
        let failures = Arc::clone(&failures);
        move |_address, _fixed| {
            failures.fetch_add(1, Ordering::Relaxed);
        }
    });

    let active = PeerImp::new(95, target, validator(95), "healthy-fixed-peer");
    assert!(overlay.activate(Arc::clone(&active)));
    assert!(active.fixed());
    assert!(matches!(
        overlay.connect(target).await,
        Err(ConnectAttemptError::DuplicateOutboundAttempt)
    ));
    assert_eq!(failures.load(Ordering::Relaxed), 0);
    drop_overlay_safely(overlay);
}

#[test]
fn outbound_attempt_registration_duplicate_ip_suppression() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 51235);

    let first = overlay
        .reserve_outbound_attempt(target)
        .expect("first attempt");
    assert!(overlay.reserve_outbound_attempt(target).is_none());
    drop(first);
    let second = overlay
        .reserve_outbound_attempt(target)
        .expect("released attempt");
    drop(second);

    let active = PeerImp::new(
        31,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 51236),
        validator(62),
        "peer-31",
    );
    assert!(overlay.activate(active));
    assert!(overlay.reserve_outbound_attempt(target).is_none());
}

#[test]
fn outbound_attempt_registration_normalizes_ipv4_mapped_addresses() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    let mapped = SocketAddr::new(
        IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x0a00, 0x0001)),
        51235,
    );
    let v4 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 51236);

    let mapped_reservation = overlay
        .reserve_outbound_attempt(mapped)
        .expect("mapped attempt");
    assert!(overlay.reserve_outbound_attempt(v4).is_none());
    drop(mapped_reservation);
    let v4_reservation = overlay.reserve_outbound_attempt(v4).expect("v4 attempt");
    drop(v4_reservation);
}

#[test]
fn refresh_membership_state_drops_excess_peers_after_reservation_loss() {
    let overlay = OverlayImpl::new(
        Setup {
            peer_limit: 1,
            peer_limit_in: Some(0),
            peer_limit_out: Some(1),
            ..test_setup()
        },
        Arc::new(TestHandoff),
    )
    .expect("overlay");
    let external = Arc::new(ExternalReservationSource::default());
    let first = peer(26, 57);
    let reserved = peer(27, 58);

    external.insert(reserved.node_public(), "wallet-backed");
    overlay.set_peer_reservation_source(external.clone());

    assert!(overlay.activate(Arc::clone(&first)));
    assert!(overlay.activate(Arc::clone(&reserved)));
    assert_eq!(overlay.size(), 2);
    assert!(reserved.reserved());

    external.erase(reserved.node_public());
    overlay.refresh_membership_state();

    assert_eq!(overlay.size(), 1);
    assert!(!reserved.reserved());
    assert!(overlay.find_peer_by_short_id(first.id()).is_some());
    assert!(overlay.find_peer_by_short_id(reserved.id()).is_none());
}

#[test]
fn resource_drop_charge_disconnects_and_deactivates_peer() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    let charged = peer(40, 80);

    assert!(overlay.activate(Arc::clone(&charged)));
    assert!(charged.lifecycle_timer_active());
    charged.charge(
        Charge::new(800_001, "resource drop test"),
        "sustained abusive peer load".to_owned(),
    );

    assert!(charged.disconnect_requested());
    assert_eq!(overlay.peer_disconnect_charges(), 1);
    assert_eq!(
        overlay.size(),
        0,
        "disconnect request must prune active peer"
    );
    assert!(!charged.lifecycle_timer_active());
    assert_eq!(overlay.peer_disconnect(), 1);
}

#[test]
fn check_tracking_updates_active_peer_tracking_state() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    let tracked = peer(12, 43);
    tracked.record_ledger(Uint256::from_u64(12), 200);
    tracked.set_ledger_range(200, 200);
    overlay.activate(Arc::clone(&tracked));

    assert!(tracked.has_range(200, 200));
    overlay.check_tracking(400);
    assert!(!tracked.has_range(200, 200));
    overlay.check_tracking(210);
    assert!(tracked.has_range(200, 200));
}

#[test]
fn finalize_connected_peer_activates_and_applies_negotiated_flags() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    let peer = peer(13, 44);
    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Protocol-Ctl",
        "compr=lz4;txrr=1;ledgerreplay=1".parse().expect("header"),
    );

    let result = overlay
        .finalize_connect_result(ConnectAttemptResult {
            peer: Arc::clone(&peer),
            response: Response::builder().status(101).body(()).expect("response"),
            negotiated_features: headers,
            session: None,
        })
        .expect("connect result should finalize");

    assert_eq!(overlay.size(), 1);
    assert!(overlay.find_peer_by_short_id(peer.id()).is_some());
    assert!(result.peer.compression_enabled());
    assert!(result.peer.tx_reduce_relay_enabled());
    assert!(result.peer.supports_feature(ProtocolFeature::LedgerReplay));
}

#[test]
fn finalize_connected_peer_sends_cached_manifests() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    overlay.set_manifests_message_provider(|| {
        Some(ProtocolMessage::new(ProtocolPayload::Manifests(
            TmManifests {
                list: vec![wire::TmManifest {
                    stobject: vec![1, 2, 3],
                }],
                ..Default::default()
            },
        )))
    });
    let peer = peer(130, 71);

    overlay
        .finalize_connect_result(ConnectAttemptResult {
            peer: Arc::clone(&peer),
            response: Response::builder().status(101).body(()).expect("response"),
            negotiated_features: HeaderMap::new(),
            session: None,
        })
        .expect("connect result should finalize");

    let messages = peer.queued_messages();
    assert_eq!(messages.len(), 1);
    assert!(matches!(
        messages[0].protocol().payload,
        ProtocolPayload::Manifests(TmManifests { ref list, .. }) if list.len() == 1
    ));
}

#[tokio::test]
async fn http_request_preserves_coalesced_overlay_read_ahead() {
    let request = b"GET / HTTP/1.1\r\nHost: example.test\r\n\r\n";
    let frame = [0_u8, 0, 0, 1, 0, 3, 0x08];
    let (mut reader, mut writer) = duplex(2048);
    writer.write_all(request).await.expect("write request");
    writer.write_all(&frame).await.expect("write frame");
    let (_stop_tx, stop_rx) = watch::channel(false);

    let (parsed, read_ahead) = read_http_request(&mut reader, stop_rx)
        .await
        .expect("read request")
        .expect("request available");

    assert_eq!(parsed.uri().path(), "/");
    assert_eq!(read_ahead, frame);
}

#[test]
fn finalize_connected_peer_rejects_unreserved_peer_when_limit_is_full() {
    let overlay = OverlayImpl::new(
        Setup {
            peer_limit: 1,
            peer_limit_in: Some(0),
            peer_limit_out: Some(1),
            ..test_setup()
        },
        Arc::new(TestHandoff),
    )
    .expect("overlay");
    assert!(overlay.activate(peer(28, 59)));

    let peer = peer(29, 60);
    let result = overlay.finalize_connect_result(ConnectAttemptResult {
        peer: Arc::clone(&peer),
        response: Response::builder().status(101).body(()).expect("response"),
        negotiated_features: HeaderMap::new(),
        session: None,
    });

    assert!(matches!(
        result,
        Err(reason) if reason == "peer limit reached for unreserved peer"
    ));
    assert_eq!(overlay.size(), 1);
    assert!(overlay.find_peer_by_short_id(peer.id()).is_none());
}

#[tokio::test]
async fn inbound_session_routes_runtime_messages_and_tracks_metrics() {
    let clock = Arc::new(ManualClock::new(Duration::from_secs(0)));
    let overlay = OverlayImpl::with_clock(
        test_setup(),
        Arc::new(TestHandoff),
        Arc::clone(&clock) as Arc<dyn Clock>,
    )
    .expect("overlay");
    let peer = peer(14, 45);
    let validator = validator(46);
    let tx_set_hash = Uint256::from_u64(99);
    let closed_hash = Uint256::from_u64(300);
    let previous_hash = Uint256::from_u64(299);

    let (local, mut remote) = duplex(4096);
    let (stop_requested, stop_rx) = watch::channel(false);
    let result = overlay
        .finalize_connect_result(ConnectAttemptResult {
            peer: Arc::clone(&peer),
            response: Response::builder().status(101).body(()).expect("response"),
            negotiated_features: HeaderMap::new(),
            session: Some(PeerSessionStarter::new(Box::new(local), stop_rx)),
        })
        .expect("connect result should finalize");
    assert!(result.session.is_none());

    let inbound_ping = Message::new(
        ProtocolMessage::new(ProtocolPayload::Ping(TmPing {
            r#type: 0,
            seq: Some(19),
            ping_time: Some(20),
            net_time: Some(21),
        })),
        None,
    );
    remote
        .write_all(inbound_ping.get_buffer(crate::Compressed::Off))
        .await
        .expect("write inbound ping");
    remote.flush().await.expect("flush inbound ping");

    let expected_pong = Message::new(
        ProtocolMessage::new(ProtocolPayload::Ping(TmPing {
            r#type: 1,
            seq: Some(19),
            ping_time: Some(20),
            net_time: Some(21),
        })),
        None,
    );
    let mut pong_bytes = vec![0u8; expected_pong.get_buffer_size()];
    timeout(Duration::from_secs(1), remote.read_exact(&mut pong_bytes))
        .await
        .expect("read pong")
        .expect("pong bytes");
    let decoded_pong = decode_protocol_message(&pong_bytes, false).expect("decode pong");
    assert!(matches!(
        decoded_pong.message,
        Some(ProtocolMessage {
            payload: ProtocolPayload::Ping(TmPing { r#type: 1, .. }),
            ..
        })
    ));

    let inbound_status = Message::new(
        ProtocolMessage::new(ProtocolPayload::StatusChange(TmStatusChange {
            new_status: Some(2),
            new_event: Some(2),
            ledger_seq: Some(300),
            ledger_hash: Some(closed_hash.data().to_vec()),
            ledger_hash_previous: Some(previous_hash.data().to_vec()),
            network_time: Some(55),
            first_seq: Some(250),
            last_seq: Some(300),
        })),
        None,
    );
    remote
        .write_all(inbound_status.get_buffer(crate::Compressed::Off))
        .await
        .expect("write inbound status");
    remote.flush().await.expect("flush inbound status");

    let inbound_have_set = Message::new(
        ProtocolMessage::new(ProtocolPayload::HaveSet(TmHaveTransactionSet {
            status: 1,
            hash: tx_set_hash.data().to_vec(),
        })),
        None,
    );
    remote
        .write_all(inbound_have_set.get_buffer(crate::Compressed::Off))
        .await
        .expect("write inbound have set");
    remote.flush().await.expect("flush inbound have set");

    let inbound_squelch = Message::new(
        ProtocolMessage::new(ProtocolPayload::Squelch(TmSquelch {
            squelch: true,
            validator_pub_key: validator.as_bytes().to_vec(),
            squelch_duration: Some(300),
        })),
        None,
    );
    remote
        .write_all(inbound_squelch.get_buffer(crate::Compressed::Off))
        .await
        .expect("write inbound squelch");
    remote.flush().await.expect("flush inbound squelch");

    let inbound_tx = Message::new(
        ProtocolMessage::new(ProtocolPayload::Transaction(TmTransaction {
            raw_transaction: vec![7; 1024],
            status: 1,
            receive_timestamp: None,
            deferred: None,
        })),
        None,
    );
    remote
        .write_all(inbound_tx.get_buffer(crate::Compressed::Off))
        .await
        .expect("write inbound transaction");
    remote.flush().await.expect("flush inbound transaction");

    clock.advance(Duration::from_secs(1));
    remote
        .write_all(inbound_tx.get_buffer(crate::Compressed::Off))
        .await
        .expect("write second inbound transaction");
    remote
        .flush()
        .await
        .expect("flush second inbound transaction");

    timeout(Duration::from_secs(1), async {
        loop {
            if peer.closed_ledger_hash() == closed_hash
                && peer.previous_ledger_hash() == previous_hash
                && peer.ledger_range() == (250, 300)
                && peer.has_tx_set(tx_set_hash)
                && peer.is_squelched(validator)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("wait for inbound routing");

    timeout(Duration::from_secs(1), async {
        loop {
            let counts = overlay.traffic.counts();
            let total = counts.get(&TrafficCategory::Total).expect("total traffic");
            let transactions = counts
                .get(&TrafficCategory::Transaction)
                .expect("transaction traffic");
            if total.messages_in.load(Ordering::Relaxed) >= 5
                && transactions.messages_in.load(Ordering::Relaxed) >= 2
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("wait for traffic metrics");

    let total = overlay
        .traffic
        .counts()
        .get(&TrafficCategory::Total)
        .expect("total traffic");
    assert!(total.bytes_in.load(Ordering::Relaxed) > 0);

    let transactions = overlay
        .traffic
        .counts()
        .get(&TrafficCategory::Transaction)
        .expect("transaction traffic");
    assert!(transactions.messages_in.load(Ordering::Relaxed) >= 2);

    let JsonValue::Object(metrics) = overlay.tx_metrics() else {
        panic!("tx metrics json");
    };
    assert_ne!(
        metrics.get("txr_tx_sz").and_then(|value| match value {
            JsonValue::String(value) => Some(value.as_str()),
            _ => None,
        }),
        Some("0")
    );

    let _ = stop_requested.send(true);
    std::thread::spawn(move || drop(overlay))
        .join()
        .expect("overlay drop");
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn inbound_status_change_preserves_status_publishes_and_skips_lost_sync() {
    let clock = Arc::new(ManualClock::new(Duration::from_secs(0)));
    let overlay = OverlayImpl::with_clock(
        test_setup(),
        Arc::new(TestHandoff),
        Arc::clone(&clock) as Arc<dyn Clock>,
    )
    .expect("overlay");
    overlay.set_validated_ledger_status_provider(|| Some((301, Duration::from_secs(119))));
    let published = Arc::new(Mutex::new(Vec::<JsonValue>::new()));
    overlay.set_peer_status_publisher({
        let published = Arc::clone(&published);
        move |payload| {
            published
                .lock()
                .expect("published peer status lock")
                .push(payload);
        }
    });

    let peer = peer(30, 70);
    let (local, mut remote) = duplex(4096);
    let (stop_requested, stop_rx) = watch::channel(false);
    overlay
        .finalize_connect_result(ConnectAttemptResult {
            peer: Arc::clone(&peer),
            response: Response::builder().status(101).body(()).expect("response"),
            negotiated_features: HeaderMap::new(),
            session: Some(PeerSessionStarter::new(Box::new(local), stop_rx)),
        })
        .expect("connect result should finalize");

    let accepted = Message::new(
        ProtocolMessage::new(ProtocolPayload::StatusChange(TmStatusChange {
            new_status: Some(2),
            new_event: Some(2),
            ledger_seq: Some(300),
            ledger_hash: Some(Uint256::from_u64(300).data().to_vec()),
            ledger_hash_previous: Some(Uint256::from_u64(299).data().to_vec()),
            network_time: Some(55),
            first_seq: Some(250),
            last_seq: Some(300),
        })),
        None,
    );
    remote
        .write_all(accepted.get_buffer(crate::Compressed::Off))
        .await
        .expect("write accepted status");
    remote.flush().await.expect("flush accepted status");

    let switched = Message::new(
        ProtocolMessage::new(ProtocolPayload::StatusChange(TmStatusChange {
            new_status: None,
            new_event: Some(3),
            ledger_seq: Some(301),
            ledger_hash: Some(Uint256::from_u64(301).data().to_vec()),
            ledger_hash_previous: Some(Uint256::from_u64(300).data().to_vec()),
            network_time: Some(56),
            first_seq: Some(400),
            last_seq: Some(300),
        })),
        None,
    );
    remote
        .write_all(switched.get_buffer(crate::Compressed::Off))
        .await
        .expect("write switched status");
    remote.flush().await.expect("flush switched status");

    timeout(Duration::from_secs(1), async {
        loop {
            if published.lock().expect("published peer status lock").len() >= 2
                && peer.closed_ledger_hash() == Uint256::from_u64(301)
                && peer.previous_ledger_hash() == Uint256::from_u64(300)
                && peer.ledger_range() == (0, 0)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("wait for published status events");

    assert!(peer.has_ledger(Uint256::from_u64(301), 0));
    assert!(peer.has_ledger(Uint256::from_u64(300), 0));
    assert_eq!(peer.tracking(), Tracking::Converged);

    let published_events = published.lock().expect("published peer status lock");
    let JsonValue::Object(first) = &published_events[0] else {
        panic!("first peer status event should be an object");
    };
    assert_eq!(
        first.get("status"),
        Some(&JsonValue::String("CONNECTED".to_owned()))
    );
    assert_eq!(
        first.get("action"),
        Some(&JsonValue::String("ACCEPTED_LEDGER".to_owned()))
    );

    let JsonValue::Object(second) = &published_events[1] else {
        panic!("second peer status event should be an object");
    };
    assert_eq!(
        second.get("status"),
        Some(&JsonValue::String("CONNECTED".to_owned()))
    );
    assert_eq!(
        second.get("action"),
        Some(&JsonValue::String("SWITCHED_LEDGER".to_owned()))
    );
    assert_eq!(
        second.get("ledger_index_min"),
        Some(&JsonValue::Unsigned(400))
    );
    assert_eq!(
        second.get("ledger_index_max"),
        Some(&JsonValue::Unsigned(300))
    );
    drop(published_events);

    let lost_sync = Message::new(
        ProtocolMessage::new(ProtocolPayload::StatusChange(TmStatusChange {
            new_status: None,
            new_event: Some(4),
            ledger_seq: None,
            ledger_hash: None,
            ledger_hash_previous: None,
            network_time: Some(57),
            first_seq: None,
            last_seq: None,
        })),
        None,
    );
    remote
        .write_all(lost_sync.get_buffer(crate::Compressed::Off))
        .await
        .expect("write lost-sync status");
    remote.flush().await.expect("flush lost-sync status");

    timeout(Duration::from_secs(1), async {
        loop {
            if peer.closed_ledger_hash().is_zero() && peer.previous_ledger_hash().is_zero() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("wait for lost sync clear");

    assert_eq!(
        published.lock().expect("published peer status lock").len(),
        2
    );

    let _ = stop_requested.send(true);
}

#[test]
fn peer_status_tracking_requires_strictly_recent_validated_ledger() {
    assert!(validated_ledger_is_recent_for_peer_tracking(
        Duration::from_secs(119)
    ));
    assert!(!validated_ledger_is_recent_for_peer_tracking(
        Duration::from_secs(120)
    ));
}

#[tokio::test]
async fn inbound_session_queues_remaining_heavy_families() {
    let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(Duration::from_secs(0)));
    let overlay = OverlayImpl::with_clock(test_setup(), Arc::new(TestHandoff), Arc::clone(&clock))
        .expect("overlay");
    let peer = peer(15, 61);
    let (local, mut remote) = duplex(64 * 1024);
    let (_stop_requested, stop_rx) = watch::channel(false);
    let mut headers = HeaderMap::new();
    headers.insert("Upgrade", "XRPL/2.2".parse().expect("upgrade header"));
    headers.insert(
        "X-Protocol-Ctl",
        "txrr=1;ledgerreplay=1".parse().expect("control header"),
    );

    let _ = overlay
        .finalize_connect_result(ConnectAttemptResult {
            peer: Arc::clone(&peer),
            response: Response::builder().status(101).body(()).expect("response"),
            negotiated_features: headers,
            session: Some(PeerSessionStarter::new(Box::new(local), stop_rx)),
        })
        .expect("connect result should finalize");

    peer.record_ledger(Uint256::from_u64(900), 900);
    peer.set_ledger_range(900, 900);
    peer.check_tracking(900);
    // This fixture uses a synthetic legacy proposal signature. Rippled
    // bypasses proposal signature verification for trusted cluster peers.
    peer.set_clustered(true);

    // Kept for compatibility with the legacy overlay wire fixtures; these
    // deprecated fields still exist on the protobuf surface we ingest.
    #[allow(deprecated)]
    let manifests = Message::new(
        ProtocolMessage::new(ProtocolPayload::Manifests(TmManifests {
            list: vec![wire::TmManifest {
                stobject: vec![1, 2, 3],
            }],
            history: None,
        })),
        None,
    );
    remote
        .write_all(manifests.get_buffer(crate::Compressed::Off))
        .await
        .expect("write manifests");

    let endpoints = Message::new(
        ProtocolMessage::new(ProtocolPayload::Endpoints(TmEndpoints {
            version: 2,
            endpoints_v2: vec![
                wire::tm_endpoints::TmEndpointv2 {
                    endpoint: "10.0.0.1:51235".to_owned(),
                    hops: 0,
                },
                wire::tm_endpoints::TmEndpointv2 {
                    endpoint: "not-an-endpoint".to_owned(),
                    hops: 2,
                },
            ],
        })),
        None,
    );
    remote
        .write_all(endpoints.get_buffer(crate::Compressed::Off))
        .await
        .expect("write endpoints");

    let single_tx = Message::new(
        ProtocolMessage::new(ProtocolPayload::Transaction(tx_message(7))),
        None,
    );
    remote
        .write_all(single_tx.get_buffer(crate::Compressed::Off))
        .await
        .expect("write transaction");

    let get_ledger = Message::new(
        ProtocolMessage::new(ProtocolPayload::GetLedger(TmGetLedger {
            itype: 0,
            ltype: Some(0),
            ledger_hash: Some(Uint256::from_u64(700).data().to_vec()),
            ledger_seq: None,
            node_i_ds: Vec::new(),
            request_cookie: None,
            query_type: None,
            query_depth: None,
        })),
        None,
    );
    remote
        .write_all(get_ledger.get_buffer(crate::Compressed::Off))
        .await
        .expect("write get ledger");

    let ledger_data = Message::new(
        ProtocolMessage::new(ProtocolPayload::LedgerData(TmLedgerData {
            ledger_hash: Uint256::from_u64(701).data().to_vec(),
            ledger_seq: 700,
            r#type: 0,
            nodes: vec![wire::TmLedgerNode {
                nodedata: vec![9, 9, 9],
                nodeid: None,
                ..wire::TmLedgerNode::default()
            }],
            request_cookie: None,
            error: None,
        })),
        None,
    );
    remote
        .write_all(ledger_data.get_buffer(crate::Compressed::Off))
        .await
        .expect("write ledger data");

    // Kept for compatibility with the legacy overlay wire fixtures; these
    // deprecated fields still exist on the protobuf surface we ingest.
    #[allow(deprecated)]
    let proposal = Message::new(
        ProtocolMessage::new(ProtocolPayload::ProposeLedger(TmProposeSet {
            propose_seq: 3,
            current_tx_hash: Uint256::from_u64(800).data().to_vec(),
            node_pub_key: validator(62).as_bytes().to_vec(),
            close_time: 44,
            signature: vec![4; 64],
            previousledger: Uint256::from_u64(799).data().to_vec(),
            added_transactions: Vec::new(),
            removed_transactions: Vec::new(),
            checked_signature: None,
            hops: None,
        })),
        None,
    );
    remote
        .write_all(proposal.get_buffer(crate::Compressed::Off))
        .await
        .expect("write proposal");

    let validation = Message::new(
        ProtocolMessage::new(ProtocolPayload::Validation(validation_message(
            63, 1000, 0xA1,
        ))),
        None,
    );
    remote
        .write_all(validation.get_buffer(crate::Compressed::Off))
        .await
        .expect("write validation");

    let validator_list = Message::new(
        ProtocolMessage::new(ProtocolPayload::ValidatorList(TmValidatorList {
            manifest: vec![1, 2],
            blob: vec![3, 4],
            signature: vec![5, 6],
            version: 1,
        })),
        None,
    );
    remote
        .write_all(validator_list.get_buffer(crate::Compressed::Off))
        .await
        .expect("write validator list");

    let validator_list_collection = Message::new(
        ProtocolMessage::new(ProtocolPayload::ValidatorListCollection(
            TmValidatorListCollection {
                version: 2,
                manifest: vec![7, 8],
                blobs: vec![wire::ValidatorBlobInfo {
                    manifest: Some(vec![9]),
                    blob: vec![10, 11],
                    signature: vec![12, 13],
                }],
            },
        )),
        None,
    );
    remote
        .write_all(validator_list_collection.get_buffer(crate::Compressed::Off))
        .await
        .expect("write validator list collection");

    let get_objects = Message::new(
        ProtocolMessage::new(ProtocolPayload::GetObjects(TmGetObjectByHash {
            r#type: wire::tm_get_object_by_hash::ObjectType::OtTransactions as i32,
            query: true,
            ledger_hash: Some(Uint256::from_u64(801).data().to_vec()),
            fat: None,
            objects: vec![wire::TmIndexedObject {
                hash: Some(Uint256::from_u64(802).data().to_vec()),
                node_id: None,
                index: None,
                data: None,
                ledger_seq: None,
            }],
        })),
        None,
    );
    remote
        .write_all(get_objects.get_buffer(crate::Compressed::Off))
        .await
        .expect("write get objects");

    let have_transactions = Message::new(
        ProtocolMessage::new(ProtocolPayload::HaveTransactions(TmHaveTransactions {
            hashes: vec![Uint256::from_u64(803).data().to_vec()],
        })),
        None,
    );
    remote
        .write_all(have_transactions.get_buffer(crate::Compressed::Off))
        .await
        .expect("write have transactions");

    let transactions = Message::new(
        ProtocolMessage::new(ProtocolPayload::Transactions(TmTransactions {
            transactions: vec![tx_message(8), tx_message(9)],
        })),
        None,
    );
    remote
        .write_all(transactions.get_buffer(crate::Compressed::Off))
        .await
        .expect("write transactions batch");

    let proof_request = Message::new(
        ProtocolMessage::new(ProtocolPayload::ProofPathRequest(TmProofPathRequest {
            key: Uint256::from_u64(810).data().to_vec(),
            ledger_hash: Uint256::from_u64(811).data().to_vec(),
            r#type: 1,
        })),
        None,
    );
    remote
        .write_all(proof_request.get_buffer(crate::Compressed::Off))
        .await
        .expect("write proof request");

    let proof_response = Message::new(
        ProtocolMessage::new(ProtocolPayload::ProofPathResponse(TmProofPathResponse {
            key: Uint256::from_u64(810).data().to_vec(),
            ledger_hash: Uint256::from_u64(811).data().to_vec(),
            r#type: 1,
            ledger_header: Some(vec![1, 2, 3]),
            path: vec![vec![4, 5, 6]],
            error: None,
        })),
        None,
    );
    remote
        .write_all(proof_response.get_buffer(crate::Compressed::Off))
        .await
        .expect("write proof response");

    let replay_request = Message::new(
        ProtocolMessage::new(ProtocolPayload::ReplayDeltaRequest(TmReplayDeltaRequest {
            ledger_hash: Uint256::from_u64(812).data().to_vec(),
        })),
        None,
    );
    remote
        .write_all(replay_request.get_buffer(crate::Compressed::Off))
        .await
        .expect("write replay request");

    let replay_response = Message::new(
        ProtocolMessage::new(ProtocolPayload::ReplayDeltaResponse(
            TmReplayDeltaResponse {
                ledger_hash: Uint256::from_u64(813).data().to_vec(),
                ledger_header: Some(vec![7, 8]),
                transaction: vec![payment_tx(10).get_serializer().data().to_vec()],
                error: None,
            },
        )),
        None,
    );
    remote
        .write_all(replay_response.get_buffer(crate::Compressed::Off))
        .await
        .expect("write replay response");
    remote.flush().await.expect("flush heavy families");

    timeout(Duration::from_secs(1), async {
        loop {
            let snapshot = overlay.queued_inbound_snapshot();
            if snapshot.manifests.len() == 1
                && snapshot.endpoints.len() == 1
                && snapshot.transactions.len() == 3
                && snapshot.get_ledgers.len() == 1
                && snapshot.ledger_data.len() == 1
                && snapshot.proposals.len() == 1
                && snapshot.validations.len() == 1
                && snapshot.validator_lists.len() == 1
                && snapshot.validator_list_collections.len() == 1
                && snapshot.get_objects.len() == 1
                && snapshot.have_transactions.len() == 1
                && snapshot.proof_path_requests.len() == 1
                && snapshot.proof_path_responses.len() == 1
                && snapshot.replay_delta_requests.len() == 1
                && snapshot.replay_delta_responses.len() == 1
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("wait for heavy families");

    let snapshot = overlay.queued_inbound_snapshot();
    assert_eq!(snapshot.manifests[0].peer_id, peer.id());
    assert_eq!(snapshot.endpoints[0].malformed, 1);
    assert_eq!(
        snapshot.endpoints[0].endpoints[0].endpoint,
        SocketAddr::new(peer.remote_address().ip(), 51235)
    );
    assert_eq!(snapshot.endpoints[0].endpoints[0].hops, 1);
    assert_eq!(
        snapshot.transactions[0].id,
        payment_tx(7).get_transaction_id()
    );
    assert!(!snapshot.transactions[0].batch);
    assert!(snapshot.transactions[1].batch);
    assert!(snapshot.transactions[2].batch);
    assert_eq!(
        snapshot.proposals[0].current_tx_hash,
        Uint256::from_u64(800)
    );
    assert_eq!(
        snapshot.have_transactions[0].hashes,
        vec![Uint256::from_u64(803)]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn inbound_endpoints_drop_excess_hops_and_cap_batch_size_peerfinder() {
    let overlay = Arc::new(OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay"));
    let secret = SecretKey::from_bytes([91u8; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    let peer = Arc::new(PeerImp::new(
        91,
        SocketAddr::new("127.0.0.1".parse().expect("ip"), 51235),
        public,
        "peer-91",
    ));
    peer.set_listener_check_state(true, true);
    peer.record_ledger(Uint256::from_u64(900), 900);
    peer.set_ledger_range(900, 900);
    peer.check_tracking(900);

    let mut router = OverlayInboundRouter {
        overlay: overlay.as_ref(),
        peer: &peer,
        ledger_data_deferred: false,
    };

    let mut endpoints_v2 = Vec::new();
    for index in 0..70u16 {
        endpoints_v2.push(wire::tm_endpoints::TmEndpointv2 {
            endpoint: format!("10.0.0.{}:{}", (index % 250) + 1, 5000 + index),
            hops: 1,
        });
    }
    endpoints_v2.push(wire::tm_endpoints::TmEndpointv2 {
        endpoint: "10.1.0.1:6000".to_owned(),
        hops: PEERFINDER_MAX_HOPS + 1,
    });

    let _ = router.on_endpoints(&TmEndpoints {
        version: 2,
        endpoints_v2,
    });

    let snapshot = overlay.queued_inbound_snapshot();
    assert_eq!(snapshot.endpoints.len(), 1);
    assert!(snapshot.endpoints[0].endpoints.len() <= PEERFINDER_MAX_ACCEPTED_ENDPOINTS);
    assert!(
        snapshot.endpoints[0]
            .endpoints
            .iter()
            .all(|endpoint| endpoint.hops <= PEERFINDER_MAX_HOPS + 1)
    );
    drop_overlay_safely(overlay);
}

#[tokio::test(flavor = "current_thread")]
async fn inbound_endpoints_rate_limit_and_dedupe_peerfinder() {
    let overlay = Arc::new(OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay"));
    let secret = SecretKey::from_bytes([92u8; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    let peer = Arc::new(PeerImp::new(
        92,
        SocketAddr::new("127.0.0.1".parse().expect("ip"), 51235),
        public,
        "peer-92",
    ));
    peer.set_listener_check_state(true, true);
    peer.record_ledger(Uint256::from_u64(901), 901);
    peer.set_ledger_range(901, 901);
    peer.check_tracking(901);

    let mut router = OverlayInboundRouter {
        overlay: overlay.as_ref(),
        peer: &peer,
        ledger_data_deferred: false,
    };

    let message = TmEndpoints {
        version: 2,
        endpoints_v2: vec![
            wire::tm_endpoints::TmEndpointv2 {
                endpoint: "10.0.0.1:51235".to_owned(),
                hops: 1,
            },
            wire::tm_endpoints::TmEndpointv2 {
                endpoint: "10.0.0.1:51235".to_owned(),
                hops: 1,
            },
            wire::tm_endpoints::TmEndpointv2 {
                endpoint: "[::]:51235".to_owned(),
                hops: 0,
            },
            wire::tm_endpoints::TmEndpointv2 {
                endpoint: "[::]:51236".to_owned(),
                hops: 0,
            },
        ],
    };

    let _ = router.on_endpoints(&message);
    let first = overlay.take_queued_inbound_snapshot();
    assert_eq!(first.endpoints.len(), 1);
    assert_eq!(first.endpoints[0].endpoints.len(), 2);
    assert!(first.endpoints[0].endpoints.iter().any(|endpoint| {
        endpoint.endpoint == SocketAddr::new(peer.remote_address().ip(), 51235)
    }));
    assert!(first.endpoints[0].endpoints.iter().any(|endpoint| {
        endpoint.endpoint == "10.0.0.1:51235".parse().expect("deduped endpoint")
    }));

    let _ = router.on_endpoints(&message);
    let second = overlay.queued_inbound_snapshot();
    assert!(second.endpoints.is_empty());
    drop_overlay_safely(overlay);
}

#[test]
fn endpoint_verification_rejects_private_loopback_and_zero_ports() {
    assert!(is_valid_peer_endpoint(
        "8.8.8.8:51235".parse().expect("public endpoint")
    ));
    assert!(!is_valid_peer_endpoint(
        "10.0.0.1:51235".parse().expect("private endpoint")
    ));
    assert!(!is_valid_peer_endpoint(
        "127.0.0.1:51235".parse().expect("loopback endpoint")
    ));
    assert!(!is_valid_peer_endpoint(
        "[::1]:51235".parse().expect("ipv6 loopback endpoint")
    ));
    assert!(!is_valid_peer_endpoint(
        "8.8.8.8:0".parse().expect("zero port endpoint")
    ));
}

#[test]
fn overlay_json_exposes_verify_endpoints_config_surface() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");

    assert!(!overlay.stats().verify_endpoints);
    match overlay.json() {
        JsonValue::Object(object) => {
            assert_eq!(
                object.get("verify_endpoints"),
                Some(&JsonValue::Bool(false))
            );
        }
        other => panic!("overlay json should be an object, got {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn inbound_neighbor_endpoint_requires_listener_check_before_acceptance() {
    let overlay = Arc::new(OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay"));
    let secret = SecretKey::from_bytes([93u8; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    let peer = Arc::new(PeerImp::new(
        93,
        SocketAddr::new("127.0.0.1".parse().expect("ip"), 51235),
        public,
        "peer-93",
    ));
    peer.set_listener_check_state(false, false);
    peer.record_ledger(Uint256::from_u64(902), 902);
    peer.set_ledger_range(902, 902);
    peer.check_tracking(902);

    let mut router = OverlayInboundRouter {
        overlay: overlay.as_ref(),
        peer: &peer,
        ledger_data_deferred: false,
    };

    let message = TmEndpoints {
        version: 2,
        endpoints_v2: vec![wire::tm_endpoints::TmEndpointv2 {
            endpoint: "[::]:51235".to_owned(),
            hops: 0,
        }],
    };

    let _ = router.on_endpoints(&message);
    assert!(overlay.take_queued_inbound_snapshot().endpoints.is_empty());

    let checked_peer = Arc::new(PeerImp::new(
        94,
        SocketAddr::new("127.0.0.1".parse().expect("ip"), 51235),
        public,
        "peer-94",
    ));
    checked_peer.set_listener_check_state(true, true);
    checked_peer.record_ledger(Uint256::from_u64(902), 902);
    checked_peer.set_ledger_range(902, 902);
    checked_peer.check_tracking(902);
    let mut checked_router = OverlayInboundRouter {
        overlay: overlay.as_ref(),
        peer: &checked_peer,
        ledger_data_deferred: false,
    };
    let _ = checked_router.on_endpoints(&message);
    let snapshot = overlay.take_queued_inbound_snapshot();
    assert_eq!(snapshot.endpoints.len(), 1);
    assert_eq!(snapshot.endpoints[0].endpoints.len(), 1);
    assert_eq!(
        snapshot.endpoints[0].endpoints[0].endpoint,
        SocketAddr::new(checked_peer.remote_address().ip(), 51235)
    );
    drop_overlay_safely(overlay);
}

#[test]
fn redirect_response_uses_filtered_discovered_endpoints_peerfinder() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    let now = SystemTime::now();

    overlay.remember_redirect_endpoint(
        SocketAddr::new("10.0.0.1".parse().expect("ip"), 51235),
        0,
        now,
    );
    overlay.remember_redirect_endpoint(
        SocketAddr::new("10.0.0.2".parse().expect("ip"), 51235),
        1,
        now,
    );
    overlay.remember_redirect_endpoint(
        SocketAddr::new("10.0.0.2".parse().expect("ip"), 51236),
        1,
        now,
    );
    overlay.remember_redirect_endpoint(
        SocketAddr::new("10.0.0.3".parse().expect("ip"), 51235),
        PEERFINDER_MAX_HOPS + 1,
        now,
    );
    overlay.remember_redirect_endpoint(
        SocketAddr::new("10.0.0.4".parse().expect("ip"), 51235),
        1,
        now - PEERFINDER_LIVE_CACHE_TTL - Duration::from_secs(1),
    );
    for index in 6..14u8 {
        overlay.remember_redirect_endpoint(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, index)), 51235),
            1,
            now,
        );
    }

    let request = Request::builder()
        .version(http::Version::HTTP_11)
        .body(())
        .expect("request");
    let (_, wire) = overlay
        .make_redirect_response(
            &request,
            SocketAddr::new("10.0.0.5".parse().expect("ip"), 51235),
        )
        .expect("redirect response");
    let body_offset = wire
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .expect("header terminator");
    let body = std::str::from_utf8(&wire[body_offset..]).expect("utf8 body");
    let json: serde_json::Value = serde_json::from_str(body).expect("json body");
    let peers = json["peer-ips"].as_array().expect("peer-ips array");

    assert!(
        !peers
            .iter()
            .any(|peer| peer.as_str() == Some("10.0.0.1:51235"))
    );
    assert!(
        !peers
            .iter()
            .any(|peer| peer.as_str() == Some("10.0.0.3:51235"))
    );
    assert!(
        !peers
            .iter()
            .any(|peer| peer.as_str() == Some("10.0.0.4:51235"))
    );
    assert!(
        !peers
            .iter()
            .any(|peer| peer.as_str() == Some("10.0.0.5:51235"))
    );
    assert_eq!(
        peers
            .iter()
            .filter(|peer| peer
                .as_str()
                .is_some_and(|text| text.starts_with("10.0.0.2:")))
            .count(),
        1
    );
    assert!(peers.len() <= PEERFINDER_REDIRECT_ENDPOINT_COUNT);
}

#[test]
fn queued_inbound_snapshot_can_be_cleared() {
    let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
    assert!(overlay.queued_inbound_snapshot().manifests.is_empty());
    overlay.clear_queued_inbound();
    assert!(overlay.queued_inbound_snapshot().transactions.is_empty());
}
