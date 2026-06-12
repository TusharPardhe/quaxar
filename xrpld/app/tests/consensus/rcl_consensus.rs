use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use app::{
    AmendmentStatus, AmendmentVote, AppRclConsensusAdaptor, AppRclConsensusOptions,
    AppRclConsensusRelay, AppRclConsensusValidationBridge, AppRclValidationStore,
    AppRclValidationsAdaptor, ApplicationRoot, FeeSetup, JobType, KnownAmendment,
    NullRclConsensusJournal, NullRclValidationJournal, RclConsensusClock, RclConsensusJournal,
    RclConsensusLedgerSource, RclConsensusMessageSink, RclConsensusModeSource,
    RclConsensusOpenLedgerSource, RclConsensusValidatorSource, RclValidationLedgerSource,
    RclValidationTrustSource, handle_new_validation_with_store,
};
use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use basics::slice::Slice;
use consensus::{
    ConsensusParms, ConsensusProposal, RclConsensusAdapter, RclCxLedger, RclCxPeerPos, RclCxTx,
    RclValidatedLedger, RclValidations, ValidationStatus, proposal_unique_id,
};
use ledger::{Fees, InboundTransactions, Ledger};
use overlay::{
    Handoff, OverlayHandoff, OverlayImpl, PeerImp, ProtocolPayload, Setup, SimplePeerSetBuilder,
};
use protocol::{
    KeyType, LedgerHashesBuilder, PublicKey, STTx, STValidation, STVector256, SecretKey, TxType,
    VF_FULL_VALIDATION, calc_node_id, derive_public_key, get_field_by_symbol, sign_digest,
    skip_keylet,
};
use rustls::RootCertStore;
use shamap::{item::SHAMapItem, mutation::MutableTree, sync::SyncTree, tree_node::SHAMapNodeType};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

#[derive(Default)]
struct MockLedgerAcceptor;

impl app::LedgerAcceptor for MockLedgerAcceptor {
    fn accept_ledger(
        &self,
        closed_seq: u32,
        _close_time: u32,
        _base_fee_drops: u64,
    ) -> Result<u32, String> {
        Ok(closed_seq.saturating_add(1))
    }

    fn consensus_built(&self, _ledger: Arc<Ledger>) -> Result<(), String> {
        Ok(())
    }

    fn consensus_closed_ledger(&self) -> Option<Arc<Ledger>> {
        None
    }

    fn consensus_previous_ledger(&self) -> Option<Arc<Ledger>> {
        None
    }
}

#[derive(Clone)]
struct FixedClock {
    now: NetClockTimePoint,
    close_time: NetClockTimePoint,
}

impl RclConsensusClock for FixedClock {
    fn now(&self) -> NetClockTimePoint {
        self.now
    }

    fn close_time(&self) -> NetClockTimePoint {
        self.close_time
    }
}

#[derive(Clone)]
struct ValidationLedgerSource {
    now: NetClockTimePoint,
}

impl ValidationLedgerSource {
    fn new(now: u32) -> Self {
        Self {
            now: NetClockTimePoint::new(now),
        }
    }
}

impl RclValidationLedgerSource for ValidationLedgerSource {
    fn now(&self) -> NetClockTimePoint {
        self.now
    }

    fn acquire_ledger(&self, _ledger_id: &Uint256) -> Option<RclValidatedLedger> {
        None
    }

    fn request_validated_ledger(&self, _ledger_id: &Uint256) {}
}

#[derive(Default)]
struct TrustSource;

impl RclValidationTrustSource for TrustSource {
    fn get_trusted_key(&self, identity: &PublicKey) -> Option<PublicKey> {
        Some(*identity)
    }

    fn get_listed_key(&self, _identity: &PublicKey) -> Option<PublicKey> {
        None
    }
}

struct NoopAcceptSink;

impl app::RclValidationAcceptanceSink for NoopAcceptSink {
    fn check_accept(&self, _hash: Uint256, _seq: u32) {}
}

#[derive(Default)]
struct ConsensusLedgerSource {
    ledgers: HashMap<Uint256, Arc<Ledger>>,
    requested: Arc<Mutex<Vec<Uint256>>>,
    valid_ledger_index: u32,
    have_validated: bool,
}

impl RclConsensusLedgerSource for ConsensusLedgerSource {
    fn acquire_consensus_ledger(&self, hash: &Uint256) -> Option<Arc<Ledger>> {
        self.ledgers.get(hash).cloned()
    }

    fn get_valid_ledger_index(&self) -> u32 {
        self.valid_ledger_index
    }

    fn have_validated(&self) -> bool {
        self.have_validated
    }

    fn request_consensus_ledger(&self, hash: &Uint256) {
        self.requested
            .lock()
            .expect("requested ledgers")
            .push(*hash);
    }
}

#[derive(Default)]
struct OpenLedgerSource;

impl RclConsensusOpenLedgerSource for OpenLedgerSource {
    fn current_open_transactions(&self) -> Vec<Arc<STTx>> {
        Vec::new()
    }

    fn has_open_transactions(&self) -> bool {
        false
    }
}

#[derive(Default)]
struct ValidatorSource {
    count: usize,
    expires: Option<u32>,
    quorum: usize,
    quorum_keys: HashSet<PublicKey>,
    trusted_master_keys: HashSet<PublicKey>,
}

impl RclConsensusValidatorSource for ValidatorSource {
    fn count(&self) -> usize {
        self.count
    }

    fn expires(&self) -> Option<u32> {
        self.expires
    }

    fn get_quorum_keys(&self) -> (usize, HashSet<PublicKey>) {
        (self.quorum, self.quorum_keys.clone())
    }

    fn get_trusted_master_keys(&self) -> HashSet<PublicKey> {
        self.trusted_master_keys.clone()
    }
}

#[derive(Default)]
struct ModeSource {
    mode: AtomicU8,
    blocked: AtomicBool,
}

impl ModeSource {
    fn new_connected() -> Self {
        Self {
            mode: AtomicU8::new(1),
            blocked: AtomicBool::new(false),
        }
    }
}

impl RclConsensusModeSource for ModeSource {
    fn operating_mode(&self) -> app::NetworkOpsOperatingMode {
        match self.mode.load(Ordering::Acquire) {
            4 => app::NetworkOpsOperatingMode::Full,
            _ => app::NetworkOpsOperatingMode::Connected,
        }
    }

    fn set_operating_mode(&self, mode: app::NetworkOpsOperatingMode) {
        self.mode.store(
            match mode {
                app::NetworkOpsOperatingMode::Disconnected => 0,
                app::NetworkOpsOperatingMode::Connected => 1,
                app::NetworkOpsOperatingMode::Syncing => 2,
                app::NetworkOpsOperatingMode::Tracking => 3,
                app::NetworkOpsOperatingMode::Full => 4,
            },
            Ordering::Release,
        );
    }

    fn is_blocked(&self) -> bool {
        self.blocked.load(Ordering::Acquire)
    }

    fn need_network_ledger(&self) -> bool {
        false
    }
}

#[derive(Default, Clone)]
struct RecordingJournal {
    entries: Arc<Mutex<Vec<(String, String)>>>,
}

impl RclConsensusJournal for RecordingJournal {
    fn trace(&self, message: &str) {
        self.entries
            .lock()
            .expect("journal entries")
            .push(("trace".to_owned(), message.to_owned()));
    }

    fn debug(&self, message: &str) {
        self.entries
            .lock()
            .expect("journal entries")
            .push(("debug".to_owned(), message.to_owned()));
    }

    fn info(&self, message: &str) {
        self.entries
            .lock()
            .expect("journal entries")
            .push(("info".to_owned(), message.to_owned()));
    }

    fn warn(&self, message: &str) {
        self.entries
            .lock()
            .expect("journal entries")
            .push(("warn".to_owned(), message.to_owned()));
    }

    fn error(&self, message: &str) {
        self.entries
            .lock()
            .expect("journal entries")
            .push(("error".to_owned(), message.to_owned()));
    }
}

fn mode_owner(state: &Arc<app::SharedNetworkOpsState>) -> app::AppNetworkOpsModeOwner {
    app::AppNetworkOpsModeOwner::new(Arc::clone(state), Arc::new(|| Duration::from_secs(0)))
}

struct CountingRunner {
    ticks: Arc<AtomicUsize>,
}

impl app::ConsensusRunner for CountingRunner {
    fn timer_tick(&self, _now: NetClockTimePoint) -> app::BoxFuture<'_, ()> {
        Box::pin(async move {
            self.ticks.fetch_add(1, Ordering::AcqRel);
        })
    }

    fn start_round(
        &self,
        _now: NetClockTimePoint,
        _prev_ledger_id: Uint256,
        _prev_ledger: RclCxLedger,
    ) -> app::BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn got_tx_set(&self, _now: NetClockTimePoint, _txset: Vec<RclCxTx>) -> app::BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn peer_proposal(
        &self,
        _now: NetClockTimePoint,
        _public_key: PublicKey,
        _signature: Vec<u8>,
        _suppression_id: Uint256,
        _proposal: ConsensusProposal<PublicKey, Uint256, Uint256>,
    ) -> app::BoxFuture<'_, bool> {
        Box::pin(async { false })
    }
}

#[derive(Default)]
struct RecordingSink {
    shared: Mutex<Vec<Arc<STTx>>>,
}

#[derive(Clone)]
struct RecordingSinkHandle(Arc<RecordingSink>);

impl RclConsensusMessageSink for RecordingSinkHandle {
    fn share_peer_position(&self, _peer_position: &consensus::RclCxPeerPos) {}

    fn propose(&self, _proposal: &consensus::ConsensusProposal<PublicKey, Uint256, Uint256>) {}

    fn share_tx_set(&self, _txset_id: Uint256, _tx_count: usize) {}

    fn share_transaction(&self, tx: Arc<STTx>) {
        self.0.shared.lock().expect("shared transactions").push(tx);
    }

    fn broadcast_validation(&self, _validation_bytes: Vec<u8>, _validator: PublicKey) {}
}

#[derive(Debug)]
struct AcceptedHandoff;

impl OverlayHandoff for AcceptedHandoff {
    fn on_handoff(&self, _request: &http::Request<()>, _remote_address: SocketAddr) -> Handoff {
        Handoff::Accepted
    }
}

fn overlay_setup() -> Setup {
    static TLS_PROVIDER: OnceLock<()> = OnceLock::new();
    TLS_PROVIDER.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });

    Setup {
        client_config: Some(Arc::new(
            rustls::ClientConfig::builder()
                .with_root_certificates(RootCertStore::empty())
                .with_no_client_auth(),
        )),
        tx_reduce_relay_enabled: false,
        ..Setup::default()
    }
}

fn peer(id: u32, seed: u8) -> Arc<PeerImp> {
    let secret = SecretKey::from_bytes([seed; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("peer public key");
    PeerImp::new(
        id,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51000 + id as u16),
        public,
        format!("peer-{id}"),
    )
}

fn sample_validator_keys() -> app::ValidatorKeys {
    let secret = SecretKey::from_bytes([9; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("validator key");
    let mut keys = app::ValidatorKeys::default();
    keys.keys = Some(app::Keys {
        master_public_key: public,
        public_key: public,
        secret_key: secret,
    });
    keys.node_id = app::calc_node_id(&public);
    keys
}

fn validator_keys_from_secret(secret: SecretKey) -> app::ValidatorKeys {
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("validator key");
    let mut keys = app::ValidatorKeys::default();
    keys.keys = Some(app::Keys {
        master_public_key: public,
        public_key: public,
        secret_key: secret,
    });
    keys.node_id = app::calc_node_id(&public);
    keys
}

fn signed_validation(
    seed: u8,
    ledger_id: Uint256,
    seq: u32,
    fill: impl FnOnce(&mut STValidation),
) -> STValidation {
    let secret = SecretKey::from_bytes([seed; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("validator public key");
    let mut validation = STValidation::new_signed(
        1_000,
        &public,
        calc_node_id(&public),
        &secret,
        |validation| {
            validation.set_field_h256(get_field_by_symbol("sfLedgerHash"), ledger_id);
            validation.set_field_u32(get_field_by_symbol("sfLedgerSequence"), seq);
            validation.set_flag(VF_FULL_VALIDATION);
            fill(validation);
        },
    )
    .expect("signed validation");
    validation.set_trusted();
    validation
}

fn payment_tx(sequence: u32, fill: u8) -> Arc<STTx> {
    Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(
            get_field_by_symbol("sfAccount"),
            protocol::AccountID::from_array([fill; 20]),
        );
        tx.set_account_id(
            get_field_by_symbol("sfDestination"),
            protocol::AccountID::from_array([fill.wrapping_add(1); 20]),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            protocol::STAmount::new_native(u64::from(sequence) + 10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            protocol::STAmount::new_native(10, false),
        );
    }))
}

fn install_skip_hashes(ledger: &mut Ledger, hashes: &[Uint256], last_ledger_sequence: u32) {
    let mut vector = STVector256::new();
    for &hash in hashes {
        vector.push_back(hash);
    }

    let mut tree = MutableTree::from_loaded_root(ledger.state_map().root(), ledger.header().seq);
    tree.add_item(
        SHAMapNodeType::AccountState,
        SHAMapItem::new(
            skip_keylet().key,
            LedgerHashesBuilder::new(vector)
                .set_last_ledger_sequence(last_ledger_sequence)
                .build(skip_keylet().key)
                .as_st_ledger_entry()
                .get_serializer()
                .data()
                .to_vec(),
        ),
    )
    .expect("skip hashes should insert");
    *ledger.state_map_mut() = SyncTree::from_root_with_type(
        tree.root(),
        ledger.state_map().map_type(),
        ledger.state_map().backed(),
        ledger.header().seq,
        ledger.state_map().state(),
    );
}

#[test]
fn public_validation_store_tracks_trusted_current_validations() {
    let adaptor =
        AppRclValidationsAdaptor::new(ValidationLedgerSource::new(1_000), NullRclValidationJournal);
    let mut validations = RclValidations::new(adaptor, ConsensusParms::default());
    let store = AppRclValidationStore::new(8);
    let ledger_id = Uint256::from_u64(55);
    let mut validation = signed_validation(1, ledger_id, 88, |_| {});

    let outcome = handle_new_validation_with_store(
        &TrustSource,
        &mut validations,
        &mut validation,
        false,
        None::<&NoopAcceptSink>,
        Some(&store),
        None::<&NullRclValidationJournal>,
    );

    assert_eq!(outcome, ValidationStatus::Current);
    assert_eq!(store.trusted_for_ledger_by_sequence(ledger_id, 88).len(), 1);
}

#[test]
fn public_rcl_consensus_adaptor_injects_fee_vote_pseudotx_from_stored_parent_validations() {
    let validations = Arc::new(Mutex::new(RclValidations::new(
        AppRclValidationsAdaptor::new(ValidationLedgerSource::new(1_000), NullRclValidationJournal),
        ConsensusParms::default(),
    )));
    let store = Arc::new(AppRclValidationStore::new(16));
    let bridge = AppRclConsensusValidationBridge::new(Arc::clone(&validations), Arc::clone(&store));
    let mut previous = Ledger::from_ledger_seq_and_close_time(256, 1_200, false);
    previous.apply_default_fees(Fees {
        base: 10,
        reserve: 200_000,
        increment: 50_000,
    });
    let previous = Arc::new(previous);
    let previous_id = *previous.header().hash.as_uint256();
    let fee_setup = FeeSetup {
        reference_fee: protocol::XRPAmount::from_drops(42),
        account_reserve: protocol::XRPAmount::from_drops(1_234_567),
        owner_reserve: protocol::XRPAmount::from_drops(7_654_321),
    };
    let mut validation = signed_validation(2, previous_id, 256, |validation| {
        validation.set_field_u64(
            get_field_by_symbol("sfBaseFee"),
            fee_setup
                .reference_fee
                .drops_as::<u64>()
                .expect("base fee fits u64"),
        );
        validation.set_field_u32(
            get_field_by_symbol("sfReserveBase"),
            fee_setup
                .account_reserve
                .drops_as::<u32>()
                .expect("reserve base fits u32"),
        );
        validation.set_field_u32(
            get_field_by_symbol("sfReserveIncrement"),
            fee_setup
                .owner_reserve
                .drops_as::<u32>()
                .expect("reserve increment fits u32"),
        );
    });
    let outcome = handle_new_validation_with_store(
        &TrustSource,
        &mut validations.lock().expect("validations mutex"),
        &mut validation,
        false,
        None::<&NoopAcceptSink>,
        Some(store.as_ref()),
        None::<&NullRclValidationJournal>,
    );
    assert_eq!(outcome, ValidationStatus::Current);

    let sink = Arc::new(RecordingSink::default());
    let mut adaptor = AppRclConsensusAdaptor::new(
        AppRclConsensusOptions::default(),
        FixedClock {
            now: NetClockTimePoint::new(1_000),
            close_time: NetClockTimePoint::new(1_001),
        },
        ConsensusLedgerSource::default(),
        OpenLedgerSource,
        bridge,
        ValidatorSource::default(),
        ModeSource::new_connected(),
        Arc::new(MockLedgerAcceptor),
        Arc::new(Mutex::new(InboundTransactions::new(Arc::new(
            SimplePeerSetBuilder::new(Vec::new()),
        )))),
        Arc::new(app::TransactionMaster::new()),
        RecordingSinkHandle(Arc::clone(&sink)),
        NullRclConsensusJournal,
        sample_validator_keys(),
        None,
        None,
        None,
    );
    adaptor.set_fee_vote(fee_setup);

    let prev = adaptor.remember_ledger(previous);
    let (txset, _) = adaptor.make_txset(&prev);
    assert_eq!(txset.len(), 1);

    adaptor.share_tx(&txset[0]);
    let shared = sink.shared.lock().expect("shared transactions");
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].get_txn_type(), TxType::FEE);
    assert_eq!(
        shared[0].get_field_u32(get_field_by_symbol("sfLedgerSequence")),
        257
    );
    assert_eq!(
        shared[0].get_field_u64(get_field_by_symbol("sfBaseFee")),
        fee_setup
            .reference_fee
            .drops_as::<u64>()
            .expect("base fee fits u64")
    );
}

#[test]
fn public_rcl_consensus_adaptor_injects_amendment_pseudotx_from_stored_parent_validations() {
    let validations = Arc::new(Mutex::new(RclValidations::new(
        AppRclValidationsAdaptor::new(ValidationLedgerSource::new(1_000), NullRclValidationJournal),
        ConsensusParms::default(),
    )));
    let store = Arc::new(AppRclValidationStore::new(16));
    let bridge = AppRclConsensusValidationBridge::new(Arc::clone(&validations), Arc::clone(&store));
    let previous = Arc::new(Ledger::from_ledger_seq_and_close_time(256, 1_200, false));
    let previous_id = *previous.header().hash.as_uint256();
    let amendment = Uint256::from_u64(0xA5);
    let amendment_table = Arc::new(AmendmentStatus::with_known_amendments(
        basics::chrono::weeks(2),
        [KnownAmendment::new(
            "featureA5",
            amendment,
            true,
            AmendmentVote::Up,
        )],
    ));
    let trusted_secret = SecretKey::from_bytes([5; 32]);
    let trusted_public =
        derive_public_key(KeyType::Secp256k1, &trusted_secret).expect("trusted validator key");

    let mut validation = signed_validation(5, previous_id, 256, |validation| {
        validation.set_field_v256(
            get_field_by_symbol("sfAmendments"),
            STVector256::from_values(get_field_by_symbol("sfAmendments"), vec![amendment]),
        );
    });
    let outcome = handle_new_validation_with_store(
        &TrustSource,
        &mut validations.lock().expect("validations mutex"),
        &mut validation,
        false,
        None::<&NoopAcceptSink>,
        Some(store.as_ref()),
        None::<&NullRclValidationJournal>,
    );
    assert_eq!(outcome, ValidationStatus::Current);

    let sink = Arc::new(RecordingSink::default());
    let mut adaptor = AppRclConsensusAdaptor::new(
        AppRclConsensusOptions::default(),
        FixedClock {
            now: NetClockTimePoint::new(1_000),
            close_time: NetClockTimePoint::new(1_001),
        },
        ConsensusLedgerSource::default(),
        OpenLedgerSource,
        bridge,
        ValidatorSource {
            trusted_master_keys: HashSet::from([trusted_public]),
            ..ValidatorSource::default()
        },
        ModeSource::new_connected(),
        Arc::new(MockLedgerAcceptor),
        Arc::new(Mutex::new(InboundTransactions::new(Arc::new(
            SimplePeerSetBuilder::new(Vec::new()),
        )))),
        Arc::new(app::TransactionMaster::new()),
        RecordingSinkHandle(Arc::clone(&sink)),
        NullRclConsensusJournal,
        sample_validator_keys(),
        None,
        None,
        None,
    );
    adaptor.set_amendment_table(Arc::clone(&amendment_table));

    let prev = adaptor.remember_ledger(previous);
    let (txset, _) = adaptor.make_txset(&prev);
    assert_eq!(txset.len(), 1);

    adaptor.share_tx(&txset[0]);
    let shared = sink.shared.lock().expect("shared transactions");
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].get_txn_type(), TxType::AMENDMENT);
    assert_eq!(
        shared[0].get_field_h256(get_field_by_symbol("sfAmendment")),
        amendment
    );
    assert_eq!(
        shared[0].get_field_u32(get_field_by_symbol("sfFlags")),
        protocol::ENABLE_AMENDMENT_GOT_MAJORITY_FLAG
    );
    assert_eq!(
        shared[0].get_field_u32(get_field_by_symbol("sfLedgerSequence")),
        257
    );
}

#[derive(Default)]
struct NegativeUnlValidationSource {
    trusted_by_sequence: HashMap<(Uint256, u32), Vec<PublicKey>>,
    keep_ranges: Arc<Mutex<Vec<(u32, u32)>>>,
}

impl app::RclConsensusValidationSource for NegativeUnlValidationSource {
    fn num_trusted_for_ledger(&self, _ledger_id: Uint256) -> usize {
        0
    }

    fn get_preferred_with_min_seq(&self, curr: RclValidatedLedger, _min_valid_seq: u32) -> Uint256 {
        curr.ledger_id
    }

    fn get_nodes_after(&self, _ledger: &RclValidatedLedger, _ledger_id: Uint256) -> usize {
        0
    }

    fn laggards(&self, _seq: u32, _trusted_keys: &mut BTreeSet<PublicKey>) -> usize {
        0
    }

    fn current_node_ids(&self) -> BTreeSet<PublicKey> {
        BTreeSet::new()
    }

    fn get_json_trie(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    fn set_seq_to_keep(&self, low: u32, high: u32) {
        self.keep_ranges
            .lock()
            .expect("keep ranges")
            .push((low, high));
    }

    fn trusted_keys_for_ledger_by_sequence(&self, ledger_id: Uint256, seq: u32) -> Vec<PublicKey> {
        self.trusted_by_sequence
            .get(&(ledger_id, seq))
            .cloned()
            .unwrap_or_default()
    }

    fn add_trusted_validation(&self, _node_id: PublicKey, _validation: &protocol::STValidation) {}
}

#[test]
fn public_rcl_consensus_adaptor_injects_negative_unl_pseudotx_from_voting_ledger_history() {
    let local_secret = SecretKey::from_bytes([7; 32]);
    let local = derive_public_key(KeyType::Secp256k1, &local_secret).expect("local validator");
    let disable_candidate = {
        let secret = SecretKey::from_bytes([8; 32]);
        derive_public_key(KeyType::Secp256k1, &secret).expect("candidate validator")
    };

    let mut previous = Ledger::from_ledger_seq_and_close_time(511, 1_200, false);
    let ancestors = (1..=256u64).map(Uint256::from_u64).collect::<Vec<_>>();
    install_skip_hashes(&mut previous, &ancestors, 510);
    let previous = Arc::new(previous);

    let next_seq = previous.header().seq + 1;
    let mut validation_source = NegativeUnlValidationSource::default();
    for i in 0..256usize {
        let ledger_id = ancestors[ancestors.len() - 1 - i];
        let seq = next_seq - 2 - u32::try_from(i).expect("history index fits");
        validation_source
            .trusted_by_sequence
            .entry((ledger_id, seq))
            .or_default()
            .push(local);
        if i < 100 {
            validation_source
                .trusted_by_sequence
                .entry((ledger_id, seq))
                .or_default()
                .push(disable_candidate);
        }
    }

    let sink = Arc::new(RecordingSink::default());
    let mut adaptor = AppRclConsensusAdaptor::new(
        AppRclConsensusOptions::default(),
        FixedClock {
            now: NetClockTimePoint::new(1_000),
            close_time: NetClockTimePoint::new(1_001),
        },
        ConsensusLedgerSource::default(),
        OpenLedgerSource,
        validation_source,
        ValidatorSource {
            count: 2,
            expires: None,
            quorum: 0,
            quorum_keys: HashSet::new(),
            trusted_master_keys: HashSet::from([local, disable_candidate]),
        },
        ModeSource::new_connected(),
        Arc::new(MockLedgerAcceptor),
        Arc::new(Mutex::new(InboundTransactions::new(Arc::new(
            SimplePeerSetBuilder::new(Vec::new()),
        )))),
        Arc::new(app::TransactionMaster::new()),
        RecordingSinkHandle(Arc::clone(&sink)),
        NullRclConsensusJournal,
        validator_keys_from_secret(local_secret),
        None,
        None,
        None,
    );

    let prev = adaptor.remember_ledger(previous);
    let (txset, _) = adaptor.make_txset(&prev);
    assert_eq!(txset.len(), 1);

    adaptor.share_tx(&txset[0]);
    let shared = sink.shared.lock().expect("shared transactions");
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].get_txn_type(), TxType::UNL_MODIFY);
    assert_eq!(
        shared[0].get_field_u8(get_field_by_symbol("sfUNLModifyDisabling")),
        1
    );
    assert_eq!(
        shared[0].get_field_vl(get_field_by_symbol("sfUNLModifyValidator")),
        disable_candidate.as_bytes()
    );
}

#[test]
fn public_rcl_consensus_relay_broadcasts_signed_local_proposals_and_records_suppression() {
    let overlay =
        Arc::new(OverlayImpl::new(overlay_setup(), Arc::new(AcceptedHandoff)).expect("overlay"));
    let first = peer(1, 31);
    let second = peer(2, 32);
    overlay.activate(Arc::clone(&first));
    overlay.activate(Arc::clone(&second));

    let mode = Arc::new(app::SharedNetworkOpsState::new(
        app::NetworkOpsOperatingMode::Full,
    ));
    let hash_router = Arc::new(xrpl_core::HashRouter::default());
    let clock: Arc<dyn app::RclConsensusClock> = Arc::new(FixedClock {
        now: NetClockTimePoint::new(1_000),
        close_time: NetClockTimePoint::new(1_001),
    });
    let keys = sample_validator_keys();
    let relay = AppRclConsensusRelay::new(
        Arc::clone(&clock),
        Arc::clone(&hash_router),
        Some(Arc::clone(&overlay)),
        mode_owner(&mode),
        keys.clone(),
        NullRclConsensusJournal,
    );

    let mut proposal = ConsensusProposal::new(
        Uint256::from_u64(10),
        3,
        Uint256::from_u64(20),
        NetClockTimePoint::new(33),
        NetClockTimePoint::new(44),
        keys.keys.as_ref().expect("validator keys").public_key,
    );
    relay.propose(&proposal);

    let expected_signature = sign_digest(
        &keys.keys.as_ref().expect("validator keys").public_key,
        &keys.keys.as_ref().expect("validator keys").secret_key,
        proposal.signing_hash(),
    )
    .expect("proposal signature");
    let expected_suppression = proposal_unique_id(
        *proposal.position(),
        *proposal.prev_ledger(),
        proposal.propose_seq(),
        proposal.close_time(),
        Slice::new(
            keys.keys
                .as_ref()
                .expect("validator keys")
                .public_key
                .as_bytes(),
        ),
        Slice::new(expected_signature.as_slice()),
    );

    assert_eq!(hash_router.entry_count(), 1);
    assert!(hash_router.should_relay(expected_suppression).is_some());

    for queued in [first.queued_messages(), second.queued_messages()] {
        assert_eq!(queued.len(), 1);
        match &queued[0].protocol().payload {
            ProtocolPayload::ProposeLedger(message) => {
                assert_eq!(message.propose_seq, 3);
                assert_eq!(message.current_tx_hash, proposal.position().data().to_vec());
                assert_eq!(
                    message.previousledger,
                    proposal.prev_ledger().data().to_vec()
                );
                assert_eq!(
                    message.node_pub_key,
                    keys.keys
                        .as_ref()
                        .expect("validator keys")
                        .public_key
                        .as_bytes()
                        .to_vec()
                );
                assert_eq!(message.signature, expected_signature);
            }
            other => panic!("expected proposal relay, got {other:?}"),
        }
    }
}

#[test]
fn public_rcl_consensus_relay_relays_peer_positions_and_drops_view_changes_to_connected() {
    let overlay =
        Arc::new(OverlayImpl::new(overlay_setup(), Arc::new(AcceptedHandoff)).expect("overlay"));
    let receiver = peer(3, 41);
    overlay.activate(Arc::clone(&receiver));

    let mode = Arc::new(app::SharedNetworkOpsState::new(
        app::NetworkOpsOperatingMode::Tracking,
    ));
    let relay = AppRclConsensusRelay::new(
        Arc::new(FixedClock {
            now: NetClockTimePoint::new(9),
            close_time: NetClockTimePoint::new(10),
        }),
        Arc::new(xrpl_core::HashRouter::default()),
        Some(Arc::clone(&overlay)),
        mode_owner(&mode),
        sample_validator_keys(),
        NullRclConsensusJournal,
    );

    let secret = SecretKey::from_bytes([55; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("peer public key");
    let mut proposal = ConsensusProposal::new(
        Uint256::from_u64(50),
        1,
        Uint256::from_u64(60),
        NetClockTimePoint::new(70),
        NetClockTimePoint::new(71),
        public,
    );
    let signature = sign_digest(&public, &secret, proposal.signing_hash()).expect("peer signature");
    let peer_position = RclCxPeerPos::new(public, signature, Uint256::from_u64(99), proposal);

    relay.share_peer_position(&peer_position);
    relay.consensus_view_change();

    assert_eq!(
        mode.operating_mode(),
        app::NetworkOpsOperatingMode::Connected
    );
    let queued = receiver.queued_messages();
    assert_eq!(queued.len(), 1);
    match &queued[0].protocol().payload {
        ProtocolPayload::ProposeLedger(message) => {
            assert_eq!(
                message.current_tx_hash,
                Uint256::from_u64(60).data().to_vec()
            );
            assert_eq!(
                message.previousledger,
                Uint256::from_u64(50).data().to_vec()
            );
        }
        other => panic!("expected relayed peer proposal, got {other:?}"),
    }
}

#[test]
fn public_rcl_consensus_relay_relays_disputed_transactions_once_per_hash_router_window() {
    let overlay =
        Arc::new(OverlayImpl::new(overlay_setup(), Arc::new(AcceptedHandoff)).expect("overlay"));
    let receiver = peer(4, 51);
    overlay.activate(Arc::clone(&receiver));

    let relay = AppRclConsensusRelay::new(
        Arc::new(FixedClock {
            now: NetClockTimePoint::new(123),
            close_time: NetClockTimePoint::new(124),
        }),
        Arc::new(xrpl_core::HashRouter::default()),
        Some(Arc::clone(&overlay)),
        mode_owner(&Arc::new(app::SharedNetworkOpsState::new(
            app::NetworkOpsOperatingMode::Connected,
        ))),
        sample_validator_keys(),
        NullRclConsensusJournal,
    );
    let tx = payment_tx(99, 0x42);

    relay.share_transaction(Arc::clone(&tx));
    relay.share_transaction(Arc::clone(&tx));

    let queued = receiver.queued_messages();
    assert_eq!(queued.len(), 1);
    match &queued[0].protocol().payload {
        ProtocolPayload::Transaction(message) => {
            assert_eq!(message.raw_transaction, tx.get_serializer().data().to_vec());
            assert_eq!(message.receive_timestamp, Some(123));
        }
        other => panic!("expected disputed transaction relay, got {other:?}"),
    }
}

#[test]
fn public_rcl_consensus_adaptor_logs_startup_and_deduplicates_missing_ledger_requests() {
    let requested = Arc::new(Mutex::new(Vec::new()));
    let journal = RecordingJournal::default();
    let mut adaptor = AppRclConsensusAdaptor::new(
        AppRclConsensusOptions::default(),
        FixedClock {
            now: NetClockTimePoint::new(1_000),
            close_time: NetClockTimePoint::new(1_001),
        },
        ConsensusLedgerSource {
            requested: Arc::clone(&requested),
            ..ConsensusLedgerSource::default()
        },
        OpenLedgerSource,
        app::AppRclConsensusValidationBridge::new(
            Arc::new(Mutex::new(RclValidations::new(
                AppRclValidationsAdaptor::new(
                    ValidationLedgerSource::new(1_000),
                    NullRclValidationJournal,
                ),
                ConsensusParms::default(),
            ))),
            Arc::new(AppRclValidationStore::new(8)),
        ),
        ValidatorSource::default(),
        ModeSource::new_connected(),
        Arc::new(MockLedgerAcceptor),
        Arc::new(Mutex::new(InboundTransactions::new(Arc::new(
            SimplePeerSetBuilder::new(Vec::new()),
        )))),
        Arc::new(app::TransactionMaster::new()),
        RecordingSinkHandle(Arc::new(RecordingSink::default())),
        journal.clone(),
        sample_validator_keys(),
        None,
        None,
        None,
    );

    let missing = Uint256::from_u64(404);
    assert!(adaptor.acquire_ledger(&missing).is_none());
    assert!(adaptor.acquire_ledger(&missing).is_none());
    assert_eq!(
        requested.lock().expect("requested ledgers").as_slice(),
        &[missing]
    );
    assert!(adaptor.consensus_cookie() != 0);

    let entries = journal.entries.lock().expect("journal entries");
    assert!(entries.iter().any(|(level, message)| {
        level == "info" && message.contains("Consensus engine started (cookie:")
    }));
    assert!(
        entries
            .iter()
            .any(|(level, message)| { level == "info" && message.contains("Validator identity:") })
    );
    assert!(
        entries
            .iter()
            .filter(|(level, message)| level == "warn" && message.contains("Need consensus ledger"))
            .count()
            == 1
    );
}

#[test]
fn public_rcl_consensus_acceptor_queues_accept_work_before_closing_the_ledger() {
    let mut root = ApplicationRoot::new(0).expect("root shell should build");
    let _ = root.attach_default_network_ops_runtime();
    let acceptor = root.clone_ledger_acceptor();

    assert_eq!(root.job_queue().get_job_count(JobType::Accept), 0);
    assert!(root.closed_ledger().is_none());

    assert!(acceptor.accept_ledger(44, 900, 10).is_ok());

    assert_eq!(root.job_queue().get_job_count(JobType::Accept), 1);
    assert!(root.closed_ledger().is_none());

    root.job_queue().run_until_idle();

    assert_eq!(root.job_queue().get_job_count(JobType::Accept), 0);
    assert_eq!(root.closed_ledger_seq(), Some(44));
    assert_eq!(root.open_ledger().current().ledger_current_index, 45);
}

#[tokio::test(flavor = "multi_thread")]
async fn public_consensus_runtime_only_ticks_when_called_by_the_owner() {
    let mut root = ApplicationRoot::new(0).expect("root shell should build");
    let network_ops_runtime = root.attach_default_network_ops_runtime();
    let runtime = app::AppConsensusRuntime::new(network_ops_runtime);
    let ticks = Arc::new(AtomicUsize::new(0));
    runtime.set_runner(Box::new(CountingRunner {
        ticks: Arc::clone(&ticks),
    }));

    app::ManagedComponent::start(&runtime).expect("consensus runtime should start");
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(ticks.load(Ordering::Acquire), 0);

    runtime.timer_tick(NetClockTimePoint::new(123), true).await;
    assert_eq!(ticks.load(Ordering::Acquire), 1);

    app::ManagedComponent::stop(&runtime);
    // Drop ApplicationRoot outside async context to avoid nested-runtime panic
    tokio::task::spawn_blocking(move || drop(root))
        .await
        .unwrap();
}
