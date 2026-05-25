use app::{
    AppLedgerMasterRuntime, AppRclValidationsAdaptor, ApplicationRoot, NullRclValidationJournal,
    RclValidationAcceptanceSink, RclValidationJournal, RclValidationLedgerSource,
    RclValidationTrustSource, ServiceRegistry, handle_new_validation, validated_ledger_from_ledger,
};
use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use consensus::{
    ConsensusParms, RclValidatedLedger, RclValidations, RclValidationsAdapter, ValidationStatus,
};
use ledger::Ledger;
use protocol::{
    KeyType, LedgerHashesBuilder, PublicKey, STValidation, STVector256, SecretKey,
    VF_FULL_VALIDATION, calc_node_id, derive_public_key, get_field_by_symbol, skip_keylet,
};
use shamap::{item::SHAMapItem, mutation::MutableTree, sync::SyncTree, tree_node::SHAMapNodeType};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct MockLedgerSource {
    now: NetClockTimePoint,
    ledgers: HashMap<Uint256, RclValidatedLedger>,
    requested: Arc<Mutex<Vec<Uint256>>>,
}

impl MockLedgerSource {
    fn new(now: u32) -> Self {
        Self {
            now: NetClockTimePoint::new(now),
            ledgers: HashMap::new(),
            requested: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl RclValidationLedgerSource for MockLedgerSource {
    fn now(&self) -> NetClockTimePoint {
        self.now
    }

    fn acquire_ledger(&self, ledger_id: &Uint256) -> Option<RclValidatedLedger> {
        self.ledgers.get(ledger_id).cloned()
    }

    fn request_validated_ledger(&self, ledger_id: &Uint256) {
        self.requested
            .lock()
            .expect("requested mutex")
            .push(*ledger_id);
    }
}

#[derive(Default)]
struct MockTrustSource {
    trusted: HashMap<PublicKey, PublicKey>,
    listed: HashMap<PublicKey, PublicKey>,
}

impl RclValidationTrustSource for MockTrustSource {
    fn get_trusted_key(&self, identity: &PublicKey) -> Option<PublicKey> {
        self.trusted.get(identity).copied()
    }

    fn get_listed_key(&self, identity: &PublicKey) -> Option<PublicKey> {
        self.listed.get(identity).copied()
    }
}

#[derive(Default)]
struct RecordingAcceptSink {
    accepted: Mutex<Vec<(Uint256, u32)>>,
}

impl RecordingAcceptSink {
    fn accepted(&self) -> Vec<(Uint256, u32)> {
        self.accepted.lock().expect("accepted mutex").clone()
    }
}

impl RclValidationAcceptanceSink for RecordingAcceptSink {
    fn check_accept(&self, hash: Uint256, seq: u32) {
        self.accepted
            .lock()
            .expect("accepted mutex")
            .push((hash, seq));
    }
}

#[derive(Clone, Default)]
struct RecordingJournal {
    entries: Arc<Mutex<Vec<(String, String)>>>,
}

impl RecordingJournal {
    fn entries(&self) -> Vec<(String, String)> {
        self.entries.lock().expect("entries mutex").clone()
    }
}

impl RclValidationJournal for RecordingJournal {
    fn trace(&self, message: &str) {
        self.entries
            .lock()
            .expect("entries mutex")
            .push(("trace".to_owned(), message.to_owned()));
    }

    fn info(&self, message: &str) {
        self.entries
            .lock()
            .expect("entries mutex")
            .push(("info".to_owned(), message.to_owned()));
    }

    fn error(&self, message: &str) {
        self.entries
            .lock()
            .expect("entries mutex")
            .push(("error".to_owned(), message.to_owned()));
    }

    fn warn(&self, message: &str) {
        self.entries
            .lock()
            .expect("entries mutex")
            .push(("warn".to_owned(), message.to_owned()));
    }
}

fn signed_validation(seed: u8, ledger_id: Uint256, seq: u32) -> (PublicKey, STValidation) {
    let secret = SecretKey::from_bytes([seed; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    let validation = STValidation::new_signed(
        1000,
        &public,
        calc_node_id(&public),
        &secret,
        |validation| {
            validation.set_field_h256(get_field_by_symbol("sfLedgerHash"), ledger_id);
            validation.set_field_u32(get_field_by_symbol("sfLedgerSequence"), seq);
            validation.set_field_u64(get_field_by_symbol("sfCookie"), 1);
            validation.set_flag(VF_FULL_VALIDATION);
        },
    )
    .expect("signed validation");
    (public, validation)
}

#[test]
fn application_root_exposes_real_validations_owner_through_service_registry() {
    let root = ApplicationRoot::new(0).expect("application root");
    let via_registry = <ApplicationRoot as ServiceRegistry>::get_validations(&root);

    assert!(std::ptr::eq(root.validations(), via_registry));
    assert!(
        via_registry
            .store()
            .trusted_for_ledger_by_sequence(Uint256::default(), 0)
            .is_empty()
    );
}

#[test]
fn application_root_validations_follow_attached_ledger_master_runtime() {
    let mut root = ApplicationRoot::new(0).expect("application root");
    let runtime = Arc::new(AppLedgerMasterRuntime::default());
    let mut ledger = Ledger::from_ledger_seq_and_close_time(701, 1_250, false);
    ledger.set_immutable(true);
    let ledger = Arc::new(ledger);
    runtime
        .ledger_master()
        .set_closed_ledger(Arc::clone(&ledger));

    let _ = root.attach_ledger_master_runtime(Arc::clone(&runtime));

    let acquired = root
        .validations()
        .validations()
        .lock()
        .expect("validations mutex")
        .adaptor_mut()
        .acquire(ledger.header().hash.as_uint256())
        .expect("attached ledger master runtime should satisfy validation ledger lookups");

    assert_eq!(acquired.ledger_id, *ledger.header().hash.as_uint256());
    assert_eq!(acquired.ledger_seq, ledger.header().seq);
}

#[test]
fn handle_new_validation_marks_trusted_validators_and_calls_check_accept() {
    let ledger_id = Uint256::from_u64(1001);
    let (public, mut validation) = signed_validation(1, ledger_id, 55);
    let mut source = MockLedgerSource::new(1000);
    source.ledgers.insert(
        ledger_id,
        RclValidatedLedger {
            ledger_id,
            ledger_seq: 55,
            ancestors: Vec::new(),
        },
    );
    let trust = MockTrustSource {
        trusted: HashMap::from([(public, public)]),
        listed: HashMap::new(),
    };
    let accept_sink = RecordingAcceptSink::default();
    let mut validations = RclValidations::new(
        AppRclValidationsAdaptor::new(source, NullRclValidationJournal),
        ConsensusParms::default(),
    );

    let outcome = handle_new_validation(
        &trust,
        &mut validations,
        &mut validation,
        false,
        Some(&accept_sink),
        None::<&RecordingJournal>,
    );

    assert_eq!(outcome, ValidationStatus::Current);
    assert!(validation.is_trusted());
    assert_eq!(accept_sink.accepted(), vec![(ledger_id, 55)]);
    assert_eq!(validations.num_trusted_for_ledger(ledger_id), 1);
}

#[test]
fn handle_new_validation_can_bypass_accept_and_traces_the_decision() {
    let ledger_id = Uint256::from_u64(1002);
    let (public, mut validation) = signed_validation(2, ledger_id, 56);
    let trust = MockTrustSource {
        trusted: HashMap::from([(public, public)]),
        listed: HashMap::new(),
    };
    let journal = RecordingJournal::default();
    let mut source = MockLedgerSource::new(1000);
    source.ledgers.insert(
        ledger_id,
        RclValidatedLedger {
            ledger_id,
            ledger_seq: 56,
            ancestors: Vec::new(),
        },
    );
    let accept_sink = RecordingAcceptSink::default();
    let mut validations = RclValidations::new(
        AppRclValidationsAdaptor::new(source, journal.clone()),
        ConsensusParms::default(),
    );

    let outcome = handle_new_validation(
        &trust,
        &mut validations,
        &mut validation,
        true,
        Some(&accept_sink),
        Some(&journal),
    );

    assert_eq!(outcome, ValidationStatus::Current);
    assert!(accept_sink.accepted().is_empty());
    assert!(journal.entries().iter().any(|entry| {
        entry
            == &(
                "trace".to_owned(),
                format!("Bypassing checkAccept for validation {ledger_id}"),
            )
    }));
}

#[test]
fn handle_new_validation_logs_byzantine_conflicts_at_trusted_severity() {
    let ledger_a = Uint256::from_u64(1003);
    let ledger_b = Uint256::from_u64(1004);
    let (public, mut first) = signed_validation(3, ledger_a, 57);
    let (_, mut second) = signed_validation(3, ledger_b, 57);
    let trust = MockTrustSource {
        trusted: HashMap::from([(public, public)]),
        listed: HashMap::new(),
    };
    let journal = RecordingJournal::default();
    let mut validations = RclValidations::new(
        AppRclValidationsAdaptor::new(MockLedgerSource::new(1000), journal.clone()),
        ConsensusParms::default(),
    );

    assert_eq!(
        handle_new_validation(
            &trust,
            &mut validations,
            &mut first,
            false,
            None::<&RecordingAcceptSink>,
            Some(&journal),
        ),
        ValidationStatus::Current
    );
    assert_eq!(
        handle_new_validation(
            &trust,
            &mut validations,
            &mut second,
            false,
            None::<&RecordingAcceptSink>,
            Some(&journal),
        ),
        ValidationStatus::Conflicting
    );
    assert!(journal.entries().iter().any(|(level, message)| {
        level == "error" && message.contains("Conflicting validation for 57")
    }));
}

#[test]
fn validated_ledger_from_real_ledger_reads_recent_skip_hashes() {
    let parent = Ledger::from_ledger_seq_and_close_time(255, 500, false);
    let mut ledger = Ledger::from_previous(&parent, 600);
    let mut tree = MutableTree::from_loaded_root(ledger.state_map().root(), ledger.header().seq);
    let mut hashes = STVector256::new();
    hashes.push_back(*parent.header().hash.as_uint256());
    tree.add_item(
        SHAMapNodeType::AccountState,
        SHAMapItem::new(
            skip_keylet().key,
            LedgerHashesBuilder::new(hashes)
                .set_last_ledger_sequence(parent.header().seq)
                .build(skip_keylet().key)
                .as_st_ledger_entry()
                .get_serializer()
                .data()
                .to_vec(),
        ),
    )
    .expect("skip list entry should insert");
    *ledger.state_map_mut() = SyncTree::from_root_with_type(
        tree.root(),
        ledger.state_map().map_type(),
        ledger.state_map().backed(),
        ledger.header().seq,
        ledger.state_map().state(),
    );
    let journal = RecordingJournal::default();

    let validated = validated_ledger_from_ledger(&ledger, &journal);

    assert_eq!(validated.ledger_seq, 256);
    assert_eq!(validated.ledger_id, *ledger.header().hash.as_uint256());
    assert_eq!(validated.ancestor(255), *parent.header().hash.as_uint256());
    assert!(journal.entries().is_empty());
}

#[test]
fn validated_ledger_from_real_ledger_warns_when_skip_hashes_are_missing() {
    let ledger = Ledger::from_ledger_seq_and_close_time(88, 700, false);
    let journal = RecordingJournal::default();

    let validated = validated_ledger_from_ledger(&ledger, &journal);

    assert_eq!(validated.ledger_seq, 88);
    assert!(validated.ancestors.is_empty());
    assert!(journal.entries().iter().any(|(level, message)| {
        level == "warn" && message.contains("missing recent ancestor hashes")
    }));
}
