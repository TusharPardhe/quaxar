#![allow(clippy::field_reassign_with_default)]
use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use consensus::{
    ConsensusParms, RclValidatedLedger, RclValidation, RclValidations, RclValidationsAdapter,
    ValidationStatus,
};
use protocol::{KeyType, PublicKey, SecretKey, derive_public_key};
use std::collections::BTreeSet;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
struct MockAdapter {
    now: Arc<Mutex<NetClockTimePoint>>,
    ledgers: HashMap<Uint256, RclValidatedLedger>,
}

impl MockAdapter {
    fn new(now: NetClockTimePoint) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
            ledgers: HashMap::new(),
        }
    }

    fn set_now(&self, now: NetClockTimePoint) {
        *self.now.lock().expect("now mutex poisoned") = now;
    }
}

impl RclValidationsAdapter for MockAdapter {
    fn now(&self) -> NetClockTimePoint {
        *self.now.lock().expect("now mutex poisoned")
    }

    fn acquire(&mut self, ledger_id: &Uint256) -> Option<RclValidatedLedger> {
        self.ledgers.get(ledger_id).cloned()
    }
}

#[derive(Clone)]
struct QueuedNowAdapter {
    now: Arc<Mutex<VecDeque<NetClockTimePoint>>>,
    ledgers: HashMap<Uint256, RclValidatedLedger>,
}

impl QueuedNowAdapter {
    fn new(times: Vec<NetClockTimePoint>) -> Self {
        Self {
            now: Arc::new(Mutex::new(times.into_iter().collect())),
            ledgers: HashMap::new(),
        }
    }
}

impl RclValidationsAdapter for QueuedNowAdapter {
    fn now(&self) -> NetClockTimePoint {
        let mut times = self.now.lock().expect("now queue poisoned");
        times
            .pop_front()
            .or_else(|| times.back().copied())
            .unwrap_or_else(|| NetClockTimePoint::new(0))
    }

    fn acquire(&mut self, ledger_id: &Uint256) -> Option<RclValidatedLedger> {
        self.ledgers.get(ledger_id).cloned()
    }
}

fn validator(seed: u8) -> PublicKey {
    let secret = SecretKey::from_bytes([seed; 32]);
    derive_public_key(KeyType::Secp256k1, &secret).expect("validator key")
}

#[allow(clippy::too_many_arguments, clippy::field_reassign_with_default)]
fn validation(
    ledger_id: Uint256,
    seq: u32,
    sign_time: u32,
    seen_time: u32,
    key: PublicKey,
    trusted: bool,
    full: bool,
    cookie: u64,
) -> RclValidation {
    RclValidation {
        ledger_id,
        seq,
        sign_time: NetClockTimePoint::new(sign_time),
        seen_time: NetClockTimePoint::new(seen_time),
        key,
        trusted,
        full,
        load_fee: None,
        cookie,
    }
}

fn validated_ledger(
    ledger_id: Uint256,
    ledger_seq: u32,
    ancestors: Vec<Uint256>,
) -> RclValidatedLedger {
    RclValidatedLedger {
        ledger_id,
        ledger_seq,
        ancestors,
    }
}

#[test]
fn rcl_validations_preserve_current_duplicate_conflict_and_acquisition_rules() {
    let ledger_id = Uint256::from_u64(42);
    let key = validator(1);
    let mut adapter = MockAdapter::new(NetClockTimePoint::new(100));
    adapter.ledgers.insert(
        ledger_id,
        RclValidatedLedger {
            ledger_id,
            ledger_seq: 10,
            ancestors: Vec::new(),
        },
    );

    let mut validations = RclValidations::new(adapter, ConsensusParms::default());
    let current = validation(ledger_id, 10, 95, 96, key, true, true, 1);

    assert_eq!(
        validations.add(key, current.clone()),
        ValidationStatus::Current
    );
    assert_eq!(validations.num_trusted_for_ledger(ledger_id), 1);
    assert_eq!(
        validations.last_ledger(key).map(|ledger| ledger.ledger_id),
        Some(ledger_id)
    );

    let multiple = validation(ledger_id, 10, 95, 96, key, true, true, 2);
    assert_eq!(validations.add(key, multiple), ValidationStatus::Multiple);

    let conflict = validation(Uint256::from_u64(43), 10, 95, 96, key, true, true, 3);
    assert_eq!(
        validations.add(key, conflict),
        ValidationStatus::Conflicting
    );
}

#[test]
fn rcl_validations_flush_stale_entries_after_expiration_and_acquire_ledgers() {
    let ledger_id = Uint256::from_u64(99);
    let key = validator(7);
    let adapter = MockAdapter::new(NetClockTimePoint::new(100));
    let parms = consensus::ConsensusParms {
        validation_valid_wall: Duration::from_secs(1),
        validation_valid_local: Duration::from_secs(1),
        validation_valid_early: Duration::from_secs(1),
        validation_set_expires: Duration::from_secs(1),
        ..ConsensusParms::default()
    };

    let mut validations = RclValidations::new(adapter.clone(), parms);
    let entry = validation(ledger_id, 20, 100, 100, key, true, true, 9);

    assert_eq!(validations.add(key, entry), ValidationStatus::Current);
    assert!(validations.last_ledger(key).is_none());

    validations.adaptor_mut().ledgers.insert(
        ledger_id,
        RclValidatedLedger {
            ledger_id,
            ledger_seq: 20,
            ancestors: Vec::new(),
        },
    );

    let trusted = validations.current_trusted();
    assert_eq!(trusted.len(), 1);
    assert_eq!(
        validations.last_ledger(key).map(|ledger| ledger.ledger_id),
        Some(ledger_id)
    );

    validations.adaptor().set_now(NetClockTimePoint::new(105));
    assert!(validations.current_trusted().is_empty());
    assert_eq!(validations.num_trusted_for_ledger(ledger_id), 0);
    assert!(validations.last_ledger(key).is_none());
}

#[test]
fn rcl_validations_use_a_single_clock_snapshot_for_stale_sweeps() {
    let ledger_id = Uint256::from_u64(1234);
    let key_one = validator(11);
    let key_two = validator(12);
    let adapter = QueuedNowAdapter::new(vec![
        NetClockTimePoint::new(100),
        NetClockTimePoint::new(100),
        NetClockTimePoint::new(100),
        NetClockTimePoint::new(100),
        NetClockTimePoint::new(100),
        NetClockTimePoint::new(101),
    ]);
    let mut parms = ConsensusParms::default();
    parms.validation_valid_wall = Duration::from_secs(0);
    parms.validation_valid_local = Duration::from_secs(0);
    parms.validation_valid_early = Duration::from_secs(0);
    parms.validation_set_expires = Duration::from_secs(1);

    let mut validations = RclValidations::new(adapter, parms);
    let first = validation(ledger_id, 20, 100, 100, key_one, true, true, 1);
    let second = validation(ledger_id, 20, 100, 100, key_two, true, true, 2);

    assert_eq!(validations.add(key_one, first), ValidationStatus::Current);
    assert_eq!(validations.add(key_two, second), ValidationStatus::Current);

    let trusted = validations.current_trusted();
    assert_eq!(trusted.len(), 2);
    assert_eq!(validations.num_trusted_for_ledger(ledger_id), 2);
}

#[test]
fn rcl_validations_keep_recent_ledger_sets_when_a_current_validation_advances() {
    let first_ledger = Uint256::from_u64(2001);
    let second_ledger = Uint256::from_u64(2002);
    let key = validator(21);
    let adapter = MockAdapter::new(NetClockTimePoint::new(100));
    let mut validations = RclValidations::new(adapter, ConsensusParms::default());

    assert_eq!(
        validations.add(
            key,
            validation(first_ledger, 20, 95, 96, key, true, true, 1)
        ),
        ValidationStatus::Current
    );
    assert_eq!(
        validations.add(
            key,
            validation(second_ledger, 21, 96, 97, key, true, true, 2)
        ),
        ValidationStatus::Current
    );

    assert_eq!(
        validations.trusted_for_ledger_by_sequence(first_ledger, 20),
        vec![key]
    );
    assert_eq!(
        validations.trusted_for_ledger_by_sequence(second_ledger, 21),
        vec![key]
    );
}

#[test]
fn rcl_validations_refresh_keep_ranges_before_validation_sets_expire() {
    let ledger_id = Uint256::from_u64(3001);
    let key = validator(31);
    let adapter = MockAdapter::new(NetClockTimePoint::new(100));
    let mut parms = ConsensusParms::default();
    parms.validation_valid_wall = Duration::from_secs(1_000);
    parms.validation_valid_local = Duration::from_secs(1_000);
    parms.validation_valid_early = Duration::from_secs(1_000);
    parms.validation_set_expires = Duration::from_secs(10);
    parms.validation_freshness = Duration::from_secs(2);

    let mut validations = RclValidations::new(adapter.clone(), parms);
    assert_eq!(
        validations.add(key, validation(ledger_id, 25, 100, 100, key, true, true, 1)),
        ValidationStatus::Current
    );
    validations.set_seq_to_keep(25, 26);

    validations.adaptor().set_now(NetClockTimePoint::new(109));
    assert_eq!(
        validations.trusted_for_ledger_by_sequence(ledger_id, 25),
        vec![key]
    );

    validations.adaptor().set_now(NetClockTimePoint::new(119));
    assert_eq!(
        validations.trusted_for_ledger_by_sequence(ledger_id, 25),
        vec![key]
    );

    validations.set_seq_to_keep(26, 27);
    validations.adaptor().set_now(NetClockTimePoint::new(130));
    assert!(
        validations
            .trusted_for_ledger_by_sequence(ledger_id, 25)
            .is_empty()
    );
}

#[test]
fn rcl_validations_preferred_ledger_and_nodes_after_follow_trie_support() {
    let key_a = validator(41);
    let key_b = validator(42);
    let key_c = validator(43);
    let ledger_a = Uint256::from_u64(4001);
    let ledger_b = Uint256::from_u64(4002);
    let ledger_c = Uint256::from_u64(4003);

    let mut adapter = MockAdapter::new(NetClockTimePoint::new(500));
    adapter.ledgers.insert(
        ledger_a,
        validated_ledger(
            ledger_a,
            3,
            vec![Uint256::from_u64(100), Uint256::from_u64(200)],
        ),
    );
    adapter.ledgers.insert(
        ledger_b,
        validated_ledger(
            ledger_b,
            3,
            vec![Uint256::from_u64(100), Uint256::from_u64(201)],
        ),
    );
    adapter.ledgers.insert(
        ledger_c,
        validated_ledger(
            ledger_c,
            4,
            vec![Uint256::from_u64(100), Uint256::from_u64(200), ledger_a],
        ),
    );

    let current = adapter.ledgers.get(&ledger_b).cloned().expect("ledger b");
    let ledger_a_view = adapter.ledgers.get(&ledger_a).cloned().expect("ledger a");
    let mut validations = RclValidations::new(adapter, ConsensusParms::default());
    assert_eq!(
        validations.add(
            key_a,
            validation(ledger_a, 3, 495, 496, key_a, true, true, 1)
        ),
        ValidationStatus::Current
    );
    assert_eq!(
        validations.add(
            key_b,
            validation(ledger_b, 3, 495, 496, key_b, true, true, 2)
        ),
        ValidationStatus::Current
    );
    assert_eq!(
        validations.add(
            key_c,
            validation(ledger_c, 4, 496, 497, key_c, true, true, 3)
        ),
        ValidationStatus::Current
    );

    assert_eq!(
        validations.get_preferred(current.clone()),
        Some((3, ledger_a))
    );
    assert_eq!(
        validations.get_preferred_with_min_seq(current.clone(), 3),
        ledger_a
    );
    assert_eq!(validations.get_nodes_after(&ledger_a_view, ledger_a), 1);
}

#[test]
fn rcl_validations_keep_prior_trusted_ledger_until_new_one_is_acquired() {
    let key = validator(51);
    let first_ledger = Uint256::from_u64(5001);
    let second_ledger = Uint256::from_u64(5002);
    let mut adapter = MockAdapter::new(NetClockTimePoint::new(700));
    adapter.ledgers.insert(
        first_ledger,
        validated_ledger(first_ledger, 10, vec![Uint256::from_u64(1); 9]),
    );

    let mut validations = RclValidations::new(adapter.clone(), ConsensusParms::default());
    assert_eq!(
        validations.add(
            key,
            validation(first_ledger, 10, 695, 696, key, true, true, 1)
        ),
        ValidationStatus::Current
    );
    assert_eq!(
        validations.last_ledger(key).map(|ledger| ledger.ledger_id),
        Some(first_ledger)
    );

    assert_eq!(
        validations.add(
            key,
            validation(second_ledger, 11, 696, 697, key, true, true, 2)
        ),
        ValidationStatus::Current
    );
    assert_eq!(
        validations.last_ledger(key).map(|ledger| ledger.ledger_id),
        Some(first_ledger)
    );

    validations.adaptor_mut().ledgers.insert(
        second_ledger,
        validated_ledger(second_ledger, 11, vec![Uint256::from_u64(1); 10]),
    );
    let _ = validations.current_trusted();
    assert_eq!(
        validations.last_ledger(key).map(|ledger| ledger.ledger_id),
        Some(second_ledger)
    );
}

#[test]
fn rcl_validations_trust_changes_update_current_sets_and_fee_views() {
    let ledger_id = Uint256::from_u64(6001);
    let key = validator(61);
    let mut validations = RclValidations::new(
        MockAdapter::new(NetClockTimePoint::new(900)),
        ConsensusParms::default(),
    );

    let mut entry = validation(ledger_id, 22, 899, 899, key, false, true, 9);
    entry.load_fee = Some(321);
    assert_eq!(validations.add(key, entry), ValidationStatus::Current);
    assert_eq!(validations.fees(ledger_id, 10), Vec::<u32>::new());

    validations.trust_changed(&BTreeSet::from([key]), &BTreeSet::new());
    assert_eq!(validations.fees(ledger_id, 10), vec![321]);
    assert_eq!(validations.current_node_ids(), BTreeSet::from([key]));

    let mut trusted_keys = BTreeSet::from([key]);
    assert_eq!(validations.laggards(23, &mut trusted_keys), 1);
    assert!(trusted_keys.is_empty());

    validations.trust_changed(&BTreeSet::new(), &BTreeSet::from([key]));
    assert_eq!(validations.fees(ledger_id, 10), Vec::<u32>::new());
}

/// C++ parity: Validations_test "Current public keys"
#[test]
fn rcl_validations_current_node_ids_tracks_active_validators() {
    let adapter = MockAdapter::new(NetClockTimePoint::new(1000));
    let mut validations = RclValidations::new(adapter, ConsensusParms::default());
    let key1 = validator(1);
    let key2 = validator(2);
    let key3 = validator(3);

    validations.trust_changed(&BTreeSet::from([key1, key2, key3]), &BTreeSet::new());

    let ledger_id = Uint256::from_u64(100);
    let v1 = validation(ledger_id, 10, 1000, 1000, key1, true, true, 1);
    let v2 = validation(ledger_id, 10, 1001, 1001, key2, true, true, 2);

    validations.add(key1, v1);
    validations.add(key2, v2);

    let node_ids = validations.current_node_ids();
    assert!(node_ids.contains(&key1));
    assert!(node_ids.contains(&key2));
    assert!(!node_ids.contains(&key3));
    assert_eq!(node_ids.len(), 2);
}

/// C++ parity: Validations_test "NumTrustedForLedger"
#[test]
fn rcl_validations_num_trusted_for_ledger_counts_correctly() {
    let adapter = MockAdapter::new(NetClockTimePoint::new(1000));
    let mut validations = RclValidations::new(adapter, ConsensusParms::default());
    let key1 = validator(1);
    let key2 = validator(2);
    let key3 = validator(3);

    validations.trust_changed(&BTreeSet::from([key1, key2, key3]), &BTreeSet::new());

    let ledger_a = Uint256::from_u64(100);
    let ledger_b = Uint256::from_u64(200);

    validations.add(
        key1,
        validation(ledger_a, 10, 1000, 1000, key1, true, true, 1),
    );
    validations.add(
        key2,
        validation(ledger_a, 10, 1001, 1001, key2, true, true, 2),
    );
    validations.add(
        key3,
        validation(ledger_b, 10, 1002, 1002, key3, true, true, 3),
    );

    assert_eq!(validations.num_trusted_for_ledger(ledger_a), 2);
    assert_eq!(validations.num_trusted_for_ledger(ledger_b), 1);
    assert_eq!(
        validations.num_trusted_for_ledger(Uint256::from_u64(999)),
        0
    );
}

/// C++ parity: Validations_test "SeqEnforcer"
#[test]
fn rcl_validations_seq_enforcer_rejects_duplicate_sequence() {
    let adapter = MockAdapter::new(NetClockTimePoint::new(1000));
    let mut validations = RclValidations::new(adapter, ConsensusParms::default());
    let key = validator(1);
    validations.trust_changed(&BTreeSet::from([key]), &BTreeSet::new());

    let ledger_a = Uint256::from_u64(100);
    let ledger_b = Uint256::from_u64(200);

    let status1 = validations.add(
        key,
        validation(ledger_a, 10, 1000, 1000, key, true, true, 1),
    );
    assert_eq!(status1, ValidationStatus::Current);

    // Same seq, different ledger — conflicting
    let status2 = validations.add(
        key,
        validation(ledger_b, 10, 1001, 1001, key, true, true, 2),
    );
    assert!(status2 == ValidationStatus::Conflicting || status2 == ValidationStatus::Stale);
}

/// C++ parity: Validations_test "By ledger functions"
#[test]
fn rcl_validations_trusted_for_ledger_returns_validators() {
    let adapter = MockAdapter::new(NetClockTimePoint::new(1000));
    let mut validations = RclValidations::new(adapter, ConsensusParms::default());
    let key1 = validator(1);
    let key2 = validator(2);
    let key3 = validator(3);

    validations.trust_changed(&BTreeSet::from([key1, key2, key3]), &BTreeSet::new());

    let ledger_id = Uint256::from_u64(100);
    validations.add(
        key1,
        validation(ledger_id, 10, 1000, 1000, key1, true, true, 1),
    );
    validations.add(
        key2,
        validation(ledger_id, 10, 1001, 1001, key2, true, true, 2),
    );

    let trusted = validations.trusted_for_ledger(ledger_id);
    assert_eq!(trusted.len(), 2);
    assert!(trusted.contains(&key1));
    assert!(trusted.contains(&key2));
}

/// C++ parity: Validations_test "fees"
#[test]
fn rcl_validations_fees_collects_from_trusted_validators() {
    let adapter = MockAdapter::new(NetClockTimePoint::new(1000));
    let mut validations = RclValidations::new(adapter, ConsensusParms::default());
    let key1 = validator(1);
    let key2 = validator(2);

    validations.trust_changed(&BTreeSet::from([key1, key2]), &BTreeSet::new());

    let ledger_id = Uint256::from_u64(100);
    validations.add(
        key1,
        validation(ledger_id, 10, 1000, 1000, key1, true, true, 1),
    );
    validations.add(
        key2,
        validation(ledger_id, 10, 1001, 1001, key2, true, true, 2),
    );

    let fees = validations.fees(ledger_id, 10);
    assert_eq!(fees.len(), 2);
}
