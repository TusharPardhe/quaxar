use std::sync::Arc;

use app::{
    CurrentLedgerState, SubmitResult, TransStatus, Transaction, TransactionCloseTimeSource,
    TransactionLoadOutcome, TransactionLoadSource, TransactionLocator, TransactionLocatorSource,
};
use basics::{base_uint::Uint256, range_set::ClosedInterval};
use protocol::{
    JsonOptions, JsonValue, STAmount, STArray, STObject, STTx, Ter, TxMeta, TxSearched, TxType,
    XRPAmount, get_field_by_symbol,
};

fn account(hex: &str) -> protocol::AccountID {
    protocol::AccountID::from_hex(hex).expect("account hex should parse")
}

fn payment_tx(sequence: u32) -> STTx {
    let source = account("1111111111111111111111111111111111111111");
    let destination = account("2222222222222222222222222222222222222222");

    STTx::new(TxType::PAYMENT, |tx| {
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

fn meta(tx_id: Uint256) -> TxMeta {
    let mut object = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    object.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    object.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 3);
    object.set_field_array(
        get_field_by_symbol("sfAffectedNodes"),
        STArray::new(get_field_by_symbol("sfAffectedNodes")),
    );
    TxMeta::from_stobject(tx_id, 9, object)
}

struct FakeLocatorSource {
    locator: TransactionLocator,
}

impl TransactionLocatorSource for FakeLocatorSource {
    fn locate_transaction(&self, _id: Uint256) -> TransactionLocator {
        self.locator
    }
}

struct FakeLoadSource {
    outcome: TransactionLoadOutcome,
    seen: std::cell::RefCell<Vec<(Uint256, Option<ClosedInterval<u32>>)>>,
}

impl TransactionLoadSource for FakeLoadSource {
    type Error = &'static str;

    fn load_transaction(
        &self,
        id: Uint256,
        range: Option<ClosedInterval<u32>>,
    ) -> Result<TransactionLoadOutcome, Self::Error> {
        self.seen.borrow_mut().push((id, range));
        Ok(self.outcome.clone())
    }
}

struct FixedCloseTimeSource {
    ledger_seq: u32,
    close_time: i64,
}

impl TransactionCloseTimeSource for FixedCloseTimeSource {
    fn close_time_for_ledger_seq(&self, ledger_seq: u32) -> Option<i64> {
        (ledger_seq == self.ledger_seq).then_some(self.close_time)
    }
}

#[test]
fn transaction_sql_status_maps_current_cpp_codes() {
    assert_eq!(
        Transaction::sql_transaction_status(Some("N")),
        TransStatus::NEW
    );
    assert_eq!(
        Transaction::sql_transaction_status(Some("C")),
        TransStatus::CONFLICTED
    );
    assert_eq!(
        Transaction::sql_transaction_status(Some("H")),
        TransStatus::HELD
    );
    assert_eq!(
        Transaction::sql_transaction_status(Some("V")),
        TransStatus::COMMITTED
    );
    assert_eq!(
        Transaction::sql_transaction_status(Some("I")),
        TransStatus::INCLUDED
    );
    assert_eq!(
        Transaction::sql_transaction_status(Some("U")),
        TransStatus::INVALID
    );
    assert_eq!(
        Transaction::sql_transaction_status(None),
        TransStatus::INVALID
    );

    #[cfg(debug_assertions)]
    assert!(std::panic::catch_unwind(|| Transaction::sql_transaction_status(Some("X"))).is_err());

    #[cfg(not(debug_assertions))]
    assert_eq!(
        Transaction::sql_transaction_status(Some("X")),
        TransStatus::INVALID
    );
}

#[test]
fn transaction_from_sql_rebuilds_status_and_ledger() {
    let tx = payment_tx(7);
    let serializer = tx.get_serializer();

    let transaction = Transaction::transaction_from_sql(Some(44), Some("V"), serializer.data())
        .expect("raw SQL transaction should parse");

    assert_eq!(transaction.get_status(), TransStatus::COMMITTED);
    assert_eq!(transaction.get_ledger(), 44);
    assert!(transaction.is_validated());
    assert_eq!(transaction.get_id(), tx.get_transaction_id());
    assert_eq!(
        transaction.get_s_transaction().get_transaction_id(),
        tx.get_transaction_id()
    );
}

#[test]
fn transaction_from_sql_rejects_out_of_range_ledger_sequence() {
    let tx = payment_tx(7);
    let serializer = tx.get_serializer();

    let error = Transaction::transaction_from_sql(
        Some(u64::from(u32::MAX) + 1),
        Some("V"),
        serializer.data(),
    )
    .expect_err("oversized ledger sequence should fail");

    assert_eq!(error, "ledger sequence exceeds u32");
}

#[test]
fn transaction_locator_matches_found_and_searched_contract() {
    let tx = payment_tx(7);
    let tx_id = tx.get_transaction_id();
    let found = Transaction::locate(
        tx_id,
        &FakeLocatorSource {
            locator: TransactionLocator::Found {
                nodestore_hash: Uint256::from_array([0xAB; 32]),
                ledger_seq: 44,
            },
        },
    );
    assert!(found.is_found());
    assert_eq!(
        found.nodestore_hash(),
        Some(Uint256::from_array([0xAB; 32]))
    );
    assert_eq!(found.ledger_sequence(), Some(44));
    assert_eq!(found.ledger_range_searched(), None);

    let searched = Transaction::locate(
        tx_id,
        &FakeLocatorSource {
            locator: TransactionLocator::Searched(ClosedInterval::new(10, 20)),
        },
    );
    assert!(!searched.is_found());
    assert_eq!(searched.nodestore_hash(), None);
    assert_eq!(searched.ledger_sequence(), None);
    assert_eq!(
        searched.ledger_range_searched(),
        Some(ClosedInterval::new(10, 20))
    );
}

#[test]
fn transaction_load_forwards_optional_range_and_txsearched_state() {
    let tx = Arc::new(payment_tx(7));
    let tx_id = tx.get_transaction_id();
    let expected_tx = Arc::new(Transaction::new(Arc::clone(&tx)));
    let expected_meta = meta(tx_id);
    let source = FakeLoadSource {
        outcome: TransactionLoadOutcome::Found {
            transaction: Arc::clone(&expected_tx),
            meta: Some(expected_meta.clone()),
        },
        seen: std::cell::RefCell::new(Vec::new()),
    };

    let loaded = Transaction::load(tx_id, &source).expect("load should succeed");
    match loaded {
        TransactionLoadOutcome::Found { transaction, meta } => {
            assert_eq!(transaction.get_id(), expected_tx.get_id());
            assert_eq!(meta, Some(expected_meta));
        }
        TransactionLoadOutcome::NotFound(state) => panic!("unexpected not found: {state:?}"),
    }
    assert_eq!(source.seen.borrow().as_slice(), &[(tx_id, None)]);

    let not_found_source = FakeLoadSource {
        outcome: TransactionLoadOutcome::NotFound(TxSearched::Some),
        seen: std::cell::RefCell::new(Vec::new()),
    };
    let range = ClosedInterval::new(30, 40);
    let loaded = Transaction::load_in_range(tx_id, &not_found_source, range)
        .expect("range load should succeed");
    match loaded {
        TransactionLoadOutcome::NotFound(state) => assert_eq!(state, TxSearched::Some),
        TransactionLoadOutcome::Found { .. } => panic!("unexpected found result"),
    }
    assert_eq!(
        not_found_source.seen.borrow().as_slice(),
        &[(tx_id, Some(range))]
    );
}

#[test]
fn transaction_submit_and_applying_flags_match_owner_contract() {
    let tx = Arc::new(payment_tx(3));
    let mut transaction = Transaction::new(tx);

    assert!(!transaction.get_applying());
    transaction.set_applying();
    assert!(transaction.get_applying());
    transaction.clear_applying();
    assert!(!transaction.get_applying());

    let initial = transaction.get_submit_result();
    assert_eq!(
        initial,
        SubmitResult {
            applied: false,
            broadcast: false,
            queued: false,
            kept: false,
        }
    );
    assert!(!initial.any());

    transaction.set_applied();
    transaction.set_broadcast();
    transaction.set_queued();
    transaction.set_kept();

    let populated = transaction.get_submit_result();
    assert!(populated.any());
    assert!(populated.applied);
    assert!(populated.broadcast);
    assert!(populated.queued);
    assert!(populated.kept);

    transaction.clear_submit_result();
    assert_eq!(
        transaction.get_submit_result(),
        SubmitResult {
            applied: false,
            broadcast: false,
            queued: false,
            kept: false,
        }
    );
}

#[test]
fn transaction_tracks_current_ledger_state_and_status_overrides() {
    let tx = Arc::new(payment_tx(9));
    let mut transaction = Transaction::new(tx);

    transaction.set_result(Ter::TES_SUCCESS);
    transaction.set_status_with_ledger(TransStatus::INCLUDED, 88, Some(12), Some(34));
    transaction.set_current_ledger_state(91, XRPAmount::from_drops(15), 16, 17);

    assert_eq!(transaction.get_result(), Ter::TES_SUCCESS);
    assert_eq!(transaction.get_status(), TransStatus::INCLUDED);
    assert_eq!(transaction.get_ledger(), 88);
    assert_eq!(
        transaction.get_current_ledger_state(),
        Some(CurrentLedgerState::new(
            91,
            XRPAmount::from_drops(15),
            16,
            17
        ))
    );
}

#[test]
fn transaction_json_adds_legacy_ledger_fields_date_and_ctid() {
    let tx = Arc::new(payment_tx(5));
    let mut transaction = Transaction::new(tx);
    transaction.set_status_with_ledger(
        TransStatus::COMMITTED,
        0x00AB_CDEF,
        Some(0x1234),
        Some(0x0042),
    );

    let JsonValue::Object(json) =
        transaction.get_json_with_close_time(JsonOptions::INCLUDE_DATE, false, Some(777))
    else {
        panic!("transaction JSON should remain an object");
    };

    assert_eq!(
        json.get("inLedger"),
        Some(&JsonValue::Unsigned(0x00AB_CDEF))
    );
    assert_eq!(
        json.get("ledger_index"),
        Some(&JsonValue::Unsigned(0x00AB_CDEF))
    );
    assert_eq!(json.get("date"), Some(&JsonValue::Signed(777)));
    assert_eq!(
        json.get("ctid"),
        Some(&JsonValue::String("C0ABCDEF12340042".to_string()))
    );
}

#[test]
fn transaction_json_omits_date_without_close_time() {
    let tx = Arc::new(payment_tx(5));
    let mut transaction = Transaction::new(tx);
    transaction.set_status_with_ledger(TransStatus::COMMITTED, 22, Some(1), Some(2));

    let JsonValue::Object(json) =
        transaction.get_json_with_close_time(JsonOptions::INCLUDE_DATE, false, None)
    else {
        panic!("transaction JSON should remain an object");
    };

    assert_eq!(json.get("ledger_index"), Some(&JsonValue::Unsigned(22)));
    assert_eq!(json.get("date"), None);
}

#[test]
fn transaction_json_can_load_date_from_owner_close_time_source() {
    let tx = Arc::new(payment_tx(5));
    let mut transaction = Transaction::new(tx);
    transaction.set_status_with_ledger(TransStatus::COMMITTED, 22, Some(1), Some(2));

    let JsonValue::Object(json) = transaction.get_json_with_close_time_source(
        JsonOptions::INCLUDE_DATE,
        false,
        &FixedCloseTimeSource {
            ledger_seq: 22,
            close_time: 654,
        },
    ) else {
        panic!("transaction JSON should remain an object");
    };

    assert_eq!(json.get("ledger_index"), Some(&JsonValue::Unsigned(22)));
    assert_eq!(json.get("date"), Some(&JsonValue::Signed(654)));
}

#[test]
fn transaction_json_uses_tx_network_id_over_owner_override() {
    let tx = Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(
            get_field_by_symbol("sfAccount"),
            account("1111111111111111111111111111111111111111"),
        );
        tx.set_account_id(
            get_field_by_symbol("sfDestination"),
            account("2222222222222222222222222222222222222222"),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 5);
        tx.set_field_u32(get_field_by_symbol("sfNetworkID"), 9);
    }));
    let mut transaction = Transaction::new(tx);
    transaction.set_status_with_ledger(TransStatus::COMMITTED, 1, Some(2), Some(3));

    let JsonValue::Object(json) = transaction.get_json(JsonOptions::NONE, false) else {
        panic!("transaction JSON should remain an object");
    };

    assert_eq!(
        json.get("ctid"),
        Some(&JsonValue::String("C000000100020009".to_string()))
    );
}

#[test]
fn transaction_status_preserves_existing_optional_ids() {
    let tx = Arc::new(payment_tx(5));
    let mut transaction = Transaction::new(tx);
    transaction.set_status_with_ledger(TransStatus::COMMITTED, 1, Some(2), Some(3));
    transaction.set_status_with_ledger(TransStatus::INCLUDED, 2, None, None);

    let JsonValue::Object(json) = transaction.get_json(JsonOptions::NONE, false) else {
        panic!("transaction JSON should remain an object");
    };

    assert_eq!(
        json.get("ctid"),
        Some(&JsonValue::String("C000000200020003".to_string()))
    );
}

#[test]
fn transaction_json_omits_ctid_when_encode_inputs_are_missing_or_out_of_range() {
    let tx = Arc::new(payment_tx(5));
    let mut oversized = Transaction::new(Arc::clone(&tx));
    oversized.set_status_with_ledger(TransStatus::COMMITTED, 0x1000_0000, Some(2), Some(3));

    let JsonValue::Object(oversized_json) = oversized.get_json(JsonOptions::NONE, false) else {
        panic!("transaction JSON should remain an object");
    };
    assert_eq!(oversized_json.get("ctid"), None);

    let mut missing = Transaction::new(tx);
    missing.set_status_with_ledger(TransStatus::COMMITTED, 1, None, Some(3));
    let JsonValue::Object(missing_json) = missing.get_json(JsonOptions::NONE, false) else {
        panic!("transaction JSON should remain an object");
    };
    assert_eq!(missing_json.get("ctid"), None);
}

#[test]
fn transaction_json_skips_legacy_field_for_v2_and_skips_binary_strings() {
    let tx = Arc::new(payment_tx(5));
    let mut transaction = Transaction::new(tx);
    transaction.set_status_with_ledger(TransStatus::COMMITTED, 22, Some(1), Some(2));

    let JsonValue::Object(v2_json) = transaction.get_json(JsonOptions::DISABLE_API_PRIOR_V2, false)
    else {
        panic!("v2 transaction JSON should remain an object");
    };
    assert_eq!(v2_json.get("inLedger"), None);
    assert_eq!(v2_json.get("ledger_index"), Some(&JsonValue::Unsigned(22)));

    let binary_v2 = transaction.get_json(JsonOptions::DISABLE_API_PRIOR_V2, true);
    match binary_v2 {
        JsonValue::String(_) => {}
        other => panic!("binary v2 JSON should remain a string, got {other:?}"),
    }
}
