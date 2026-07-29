use app::{ApplicationRoot, NetworkOpsValidationPublisher, validation_received_json};
use basics::base_uint::Uint256;
use basics::str_hex::str_hex;
use protocol::{
    JsonValue, KeyType, PublicKey, STAmount, STValidation, STVector256, SecretKey,
    VF_FULL_VALIDATION, calc_node_id, derive_public_key, get_field_by_symbol,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

#[derive(Default)]
struct RecordingValidationPublisher {
    messages: Mutex<Vec<JsonValue>>,
}

impl RecordingValidationPublisher {
    fn messages(&self) -> Vec<JsonValue> {
        self.messages.lock().expect("messages mutex").clone()
    }
}

impl NetworkOpsValidationPublisher for RecordingValidationPublisher {
    fn publish_validation(&self, message: JsonValue) {
        self.messages.lock().expect("messages mutex").push(message);
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

impl app::RclValidationAcceptanceSink for RecordingAcceptSink {
    fn check_accept(&self, hash: Uint256, seq: u32) {
        self.accepted
            .lock()
            .expect("accepted mutex")
            .push((hash, seq));
    }
}

struct BlockingAcceptSink {
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl app::RclValidationAcceptanceSink for BlockingAcceptSink {
    fn check_accept(&self, _hash: Uint256, _seq: u32) {
        self.entered
            .send(())
            .expect("signal first validation acceptance");
        self.release
            .lock()
            .expect("release mutex")
            .recv()
            .expect("release first validation acceptance");
    }
}

fn signed_validation_with_fill(
    seed: u8,
    ledger_id: Uint256,
    seq: u32,
    signing_time: u32,
    fill: impl FnOnce(&mut STValidation),
) -> (PublicKey, STValidation) {
    let secret = SecretKey::from_bytes([seed; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    let validation = STValidation::new_signed(
        signing_time,
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
    (public, validation)
}

#[test]
fn networkops_pub_validation_json_matches_current_cpp_shape_and_drop_overrides() {
    let ledger_id = Uint256::from_u64(0xA55A);
    let validated_hash = Uint256::from_u64(0xBEEFu64);
    let amendment_a = Uint256::from_u64(11);
    let amendment_b = Uint256::from_u64(12);
    let (public, validation) = signed_validation_with_fill(7, ledger_id, 55, 1000, |validation| {
        validation.set_field_u64(get_field_by_symbol("sfCookie"), 19);
        validation.set_field_u64(get_field_by_symbol("sfServerVersion"), 77);
        validation.set_field_h256(get_field_by_symbol("sfValidatedHash"), validated_hash);
        validation.set_field_u32(get_field_by_symbol("sfCloseTime"), 901);
        validation.set_field_u32(get_field_by_symbol("sfLoadFee"), 12);
        validation.set_field_u64(get_field_by_symbol("sfBaseFee"), 99);
        validation.set_field_u32(get_field_by_symbol("sfReserveBase"), 200);
        validation.set_field_u32(get_field_by_symbol("sfReserveIncrement"), 20);
        validation.set_field_amount(
            get_field_by_symbol("sfBaseFeeDrops"),
            STAmount::new_native(15, false),
        );
        validation.set_field_amount(
            get_field_by_symbol("sfReserveBaseDrops"),
            STAmount::new_native(20, false),
        );
        validation.set_field_amount(
            get_field_by_symbol("sfReserveIncrementDrops"),
            STAmount::new_native(5, false),
        );
        validation.set_field_v256(
            get_field_by_symbol("sfAmendments"),
            STVector256::from_values(
                get_field_by_symbol("sfAmendments"),
                vec![amendment_a, amendment_b],
            ),
        );
    });

    let json = validation_received_json(&validation, 1_025, None);

    assert_eq!(
        json,
        JsonValue::Object(BTreeMap::from([
            (
                "type".to_owned(),
                JsonValue::String("validationReceived".to_owned()),
            ),
            (
                "validation_public_key".to_owned(),
                JsonValue::String(public.to_node_public_base58()),
            ),
            (
                "ledger_hash".to_owned(),
                JsonValue::String(ledger_id.to_string()),
            ),
            (
                "signature".to_owned(),
                JsonValue::String(str_hex(validation.get_signature())),
            ),
            ("full".to_owned(), JsonValue::Bool(true)),
            (
                "flags".to_owned(),
                JsonValue::Unsigned(u64::from(validation.get_flags())),
            ),
            ("signing_time".to_owned(), JsonValue::Unsigned(1000)),
            (
                "data".to_owned(),
                JsonValue::String(str_hex(validation.get_serializer().data())),
            ),
            ("network_id".to_owned(), JsonValue::Unsigned(1_025)),
            (
                "server_version".to_owned(),
                JsonValue::String("77".to_owned()),
            ),
            ("cookie".to_owned(), JsonValue::String("19".to_owned())),
            (
                "validated_hash".to_owned(),
                JsonValue::String(validated_hash.to_string()),
            ),
            ("ledger_index".to_owned(), JsonValue::Unsigned(55)),
            (
                "amendments".to_owned(),
                JsonValue::Array(vec![
                    JsonValue::String(amendment_a.to_string()),
                    JsonValue::String(amendment_b.to_string()),
                ]),
            ),
            ("close_time".to_owned(), JsonValue::Unsigned(901)),
            ("load_fee".to_owned(), JsonValue::Unsigned(12)),
            ("base_fee".to_owned(), JsonValue::Signed(15)),
            ("reserve_base".to_owned(), JsonValue::Signed(20)),
            ("reserve_inc".to_owned(), JsonValue::Signed(5)),
        ]))
    );
}

#[test]
fn networkops_recv_validation_rechecks_accept_for_each_current_trusted_arrival() {
    let mut root = ApplicationRoot::new(0).expect("application root");
    let runtime = root.attach_default_network_ops_validation_runtime();
    let publisher = Arc::new(RecordingValidationPublisher::default());
    let _ = runtime.set_publisher(Some(
        Arc::clone(&publisher) as Arc<dyn NetworkOpsValidationPublisher>
    ));

    let ledger_id = Uint256::from_u64(777);
    let signing_time = root.shared_time_keeper().close_time().as_seconds();
    let (first_public, mut first_validation) =
        signed_validation_with_fill(1, ledger_id, 90, signing_time, |_| {});
    let (second_public, mut second_validation) =
        signed_validation_with_fill(2, ledger_id, 90, signing_time, |_| {});
    assert!(root.validators().load(
        None,
        &[
            first_public.to_node_public_base58(),
            second_public.to_node_public_base58(),
        ],
        &[],
        None,
    ));
    root.validators()
        .update_trusted(&std::collections::HashSet::new(), 0);
    assert!(root.validators().trusted(first_public));
    assert!(root.validators().trusted(second_public));

    let accept_sink = RecordingAcceptSink::default();
    let first_report =
        runtime.receive_validation_with_accept(&mut first_validation, "peer-1", Some(&accept_sink));
    let second_report = runtime.receive_validation_with_accept(
        &mut second_validation,
        "peer-2",
        Some(&accept_sink),
    );

    assert!(!first_report.bypass_accept);
    assert!(!second_report.bypass_accept);
    assert!(second_report.relay);
    assert!(second_report.published);
    assert!(second_report.trusted);
    assert_eq!(
        accept_sink.accepted(),
        vec![(ledger_id, 90), (ledger_id, 90)]
    );
    assert_eq!(runtime.pending_validation_count(), 0);
    assert_eq!(publisher.messages().len(), 2);

    root.validators()
        .set_negative_unl([first_public].into_iter().collect());
    root.validators()
        .update_trusted(&std::collections::HashSet::new(), 0);
    assert_eq!(root.trusted_validation_count_for_ledger(ledger_id, 90), 1);
}

#[test]
fn networkops_recv_validation_bypasses_accept_for_concurrent_same_ledger() {
    let mut root = ApplicationRoot::new(0).expect("application root");
    let runtime = root.attach_default_network_ops_validation_runtime();
    let ledger_id = Uint256::from_u64(778);
    let signing_time = root.shared_time_keeper().close_time().as_seconds();
    let (first_public, first_validation) =
        signed_validation_with_fill(3, ledger_id, 91, signing_time, |_| {});
    let (second_public, mut second_validation) =
        signed_validation_with_fill(4, ledger_id, 91, signing_time, |_| {});
    assert!(root.validators().load(
        None,
        &[
            first_public.to_node_public_base58(),
            second_public.to_node_public_base58(),
        ],
        &[],
        None,
    ));
    root.validators()
        .update_trusted(&std::collections::HashSet::new(), 0);

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let blocking_sink = Arc::new(BlockingAcceptSink {
        entered: entered_tx,
        release: Mutex::new(release_rx),
    });
    let first_runtime = Arc::clone(&runtime);
    let first_sink = Arc::clone(&blocking_sink);
    let first = thread::spawn(move || {
        let mut validation = first_validation;
        first_runtime.receive_validation_with_accept(&mut validation, "peer-1", Some(&*first_sink))
    });
    entered_rx
        .recv()
        .expect("first validation must remain pending while acceptance blocks");

    let second_sink = RecordingAcceptSink::default();
    let second_report = runtime.receive_validation_with_accept(
        &mut second_validation,
        "peer-2",
        Some(&second_sink),
    );
    assert!(second_report.bypass_accept);
    assert_eq!(runtime.pending_validation_count(), 1);
    assert!(second_sink.accepted().is_empty());

    release_tx.send(()).expect("release first validation");
    let first_report = first.join().expect("first validation thread");
    assert!(!first_report.bypass_accept);
    assert_eq!(runtime.pending_validation_count(), 0);
}

#[test]
fn application_root_owns_validation_runtime_and_updates_untrusted_relay_policy() {
    let mut root = ApplicationRoot::new(0).expect("application root");
    assert!(root.network_ops_validation_runtime().is_none());

    let runtime = root.attach_default_network_ops_validation_runtime();
    assert!(root.network_ops_validation_runtime().is_some());
    assert!(root.relay_untrusted_validations());
    assert!(runtime.relay_untrusted_validations());

    let publisher = Arc::new(RecordingValidationPublisher::default());
    let _ = runtime.set_publisher(Some(
        Arc::clone(&publisher) as Arc<dyn NetworkOpsValidationPublisher>
    ));

    let (_, mut first_validation) =
        signed_validation_with_fill(9, Uint256::from_u64(901), 101, 1000, |_| {});
    first_validation.set_untrusted();
    let first_report = root
        .receive_validation_to_network_ops(&mut first_validation, "peer-a")
        .expect("validation runtime attached");
    assert!(first_report.relay);
    assert!(first_report.published);
    assert_eq!(root.network_ops_pending_validation_count(), Some(0));

    let previous = root.set_relay_untrusted_validations(false);
    assert!(previous);
    assert!(!root.relay_untrusted_validations());
    assert!(!runtime.relay_untrusted_validations());

    let (_, mut second_validation) =
        signed_validation_with_fill(10, Uint256::from_u64(902), 102, 1000, |_| {});
    second_validation.set_untrusted();
    let second_report = root
        .receive_validation_to_network_ops(&mut second_validation, "peer-b")
        .expect("validation runtime attached");
    assert!(!second_report.relay);
    assert!(second_report.published);
    assert_eq!(publisher.messages().len(), 2);
}
