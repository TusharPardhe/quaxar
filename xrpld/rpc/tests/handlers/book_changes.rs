//! Tests for the book changes RPC handler.

use std::{collections::BTreeMap, time::Duration};

use basics::base_uint::Uint256;
use protocol::{
    AccountID, IOUAmount, Issue, JsonValue, LedgerEntryType, STAmount, STArray, STObject, STTx,
    TxMeta, TxType, currency_from_string, get_field_by_symbol,
};
use rpc::{
    BookChangesLedger, BookChangesRequest, BookChangesSource, BookChangesTransaction,
    LedgerLookupLedger, LedgerLookupSource, RpcRole, do_book_changes,
};

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn issue(code: &str, issuer: u8) -> Issue {
    Issue::new(currency_from_string(code), account(issuer))
}

fn amount(field: &'static protocol::SField, issue: Issue, mantissa: i64) -> STAmount {
    STAmount::from_iou_amount(
        field,
        IOUAmount::from_parts(mantissa, -6).expect("IOU amount should normalize"),
        issue,
    )
}

fn payment_tx(sequence: u32) -> STTx {
    STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(0x10));
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(0x20));
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

fn offer_change_meta(domain: Option<Uint256>) -> TxMeta {
    let usd = issue("USD", 0x31);
    let eur = issue("EUR", 0x32);

    let mut previous = STObject::new(get_field_by_symbol("sfPreviousFields"));
    previous.set_field_amount(
        get_field_by_symbol("sfTakerGets"),
        amount(get_field_by_symbol("sfTakerGets"), usd, 70),
    );
    previous.set_field_amount(
        get_field_by_symbol("sfTakerPays"),
        amount(get_field_by_symbol("sfTakerPays"), eur, 140),
    );

    let mut final_fields = STObject::new(get_field_by_symbol("sfFinalFields"));
    final_fields.set_field_amount(
        get_field_by_symbol("sfTakerGets"),
        amount(get_field_by_symbol("sfTakerGets"), usd, 40),
    );
    final_fields.set_field_amount(
        get_field_by_symbol("sfTakerPays"),
        amount(get_field_by_symbol("sfTakerPays"), eur, 80),
    );
    final_fields.set_field_u32(get_field_by_symbol("sfSequence"), 9);
    if let Some(domain) = domain {
        final_fields.set_field_h256(get_field_by_symbol("sfDomainID"), domain);
    }

    let mut node = STObject::new(get_field_by_symbol("sfModifiedNode"));
    node.set_field_u16(
        get_field_by_symbol("sfLedgerEntryType"),
        LedgerEntryType::Offer.code(),
    );
    node.set_field_object(get_field_by_symbol("sfPreviousFields"), previous);
    node.set_field_object(get_field_by_symbol("sfFinalFields"), final_fields);

    let mut affected = STArray::new(get_field_by_symbol("sfAffectedNodes"));
    affected.push_back(node);

    let mut object = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    object.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    object.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 1);
    object.set_field_array(get_field_by_symbol("sfAffectedNodes"), affected);
    TxMeta::from_stobject(Uint256::from_array([0x41; 32]), 90, object)
}

#[derive(Clone)]
struct TestSource {
    current: Option<LedgerLookupLedger>,
    closed: Option<LedgerLookupLedger>,
    validated: Option<LedgerLookupLedger>,
    by_seq: BTreeMap<u32, LedgerLookupLedger>,
    by_hash: BTreeMap<Uint256, LedgerLookupLedger>,
    valid_ledger_index: u32,
    validated_age: Duration,
    snapshots: BTreeMap<u32, BookChangesLedger>,
}

impl LedgerLookupSource for TestSource {
    fn get_ledger_by_hash(&self, hash: Uint256) -> Option<LedgerLookupLedger> {
        self.by_hash.get(&hash).copied()
    }

    fn get_ledger_by_seq(&self, seq: u32) -> Option<LedgerLookupLedger> {
        self.by_seq.get(&seq).copied()
    }

    fn get_current_ledger(&self) -> Option<LedgerLookupLedger> {
        self.current
    }

    fn get_closed_ledger(&self) -> Option<LedgerLookupLedger> {
        self.closed
    }

    fn get_validated_ledger(&self) -> Option<LedgerLookupLedger> {
        self.validated
    }

    fn get_valid_ledger_index(&self) -> u32 {
        self.valid_ledger_index
    }

    fn get_validated_ledger_age(&self) -> Duration {
        self.validated_age
    }

    fn is_validated(&self, ledger: &LedgerLookupLedger) -> bool {
        self.validated == Some(*ledger)
    }
}

impl BookChangesSource for TestSource {
    fn book_changes_ledger(&self, ledger: LedgerLookupLedger) -> Option<BookChangesLedger> {
        self.snapshots.get(&ledger.seq).cloned()
    }
}

fn source() -> TestSource {
    let current = LedgerLookupLedger {
        hash: Uint256::from_array([0x11; 32]),
        seq: 200,
        open: true,
    };
    let validated = LedgerLookupLedger {
        hash: Uint256::from_array([0x22; 32]),
        seq: 199,
        open: false,
    };

    let txn = BookChangesTransaction {
        txn: payment_tx(1),
        meta: offer_change_meta(Some(Uint256::from_array([0x55; 32]))),
    };

    TestSource {
        current: Some(current),
        closed: Some(validated),
        validated: Some(validated),
        by_seq: BTreeMap::from([(199, validated)]),
        by_hash: BTreeMap::from([(validated.hash, validated)]),
        valid_ledger_index: 200,
        validated_age: Duration::from_secs(30),
        snapshots: BTreeMap::from([
            (
                200,
                BookChangesLedger {
                    ledger_time: 777,
                    transactions: vec![txn.clone()],
                },
            ),
            (
                199,
                BookChangesLedger {
                    ledger_time: 666,
                    transactions: vec![txn],
                },
            ),
        ]),
    }
}

#[test]
fn book_changes_defaults_to_current_ledger_shape() {
    let source = source();
    let params = object([]);

    let result = do_book_changes(
        &BookChangesRequest {
            params: &params,
            api_version: 2,
            role: RpcRole::Guest,
        },
        &source,
    );

    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("ledger_current_index"),
        Some(&JsonValue::Unsigned(200))
    );
    assert_eq!(result.get("validated"), Some(&JsonValue::Bool(false)));
    assert_eq!(
        result.get("type"),
        Some(&JsonValue::String("bookChanges".to_owned()))
    );
    assert_eq!(result.get("ledger_time"), Some(&JsonValue::Unsigned(777)));
}

#[test]
fn book_changes_preserves_domain_and_validated_flag() {
    let source = source();
    let params = object([("ledger_index", JsonValue::String("validated".to_owned()))]);

    let result = do_book_changes(
        &BookChangesRequest {
            params: &params,
            api_version: 2,
            role: RpcRole::Guest,
        },
        &source,
    );

    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("validated"), Some(&JsonValue::Bool(true)));
    let JsonValue::Array(changes) = result.get("changes").expect("changes should be present")
    else {
        panic!("changes must be an array");
    };
    assert_eq!(changes.len(), 1);
    let JsonValue::Object(change) = &changes[0] else {
        panic!("change entry must be an object");
    };
    assert_eq!(
        change.get("domain"),
        Some(&JsonValue::String(
            Uint256::from_array([0x55; 32]).to_string()
        ))
    );
}

#[test]
fn book_changes_change_entry_has_expected_fields() {
    let source = source();
    let params = object([("ledger_index", JsonValue::String("validated".to_owned()))]);

    let result = do_book_changes(
        &BookChangesRequest {
            params: &params,
            api_version: 2,
            role: RpcRole::Guest,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Array(changes) = result.get("changes").expect("changes") else {
        panic!("changes must be an array");
    };
    assert_eq!(changes.len(), 1);

    let JsonValue::Object(change) = &changes[0] else {
        panic!("change must be an object");
    };
    // Each change should have currency_a, currency_b, volume_a, volume_b, high, low, open, close
    assert!(change.contains_key("currency_a"), "should have currency_a");
    assert!(change.contains_key("currency_b"), "should have currency_b");
    assert!(change.contains_key("volume_a"), "should have volume_a");
    assert!(change.contains_key("volume_b"), "should have volume_b");
    assert!(change.contains_key("high"), "should have high");
    assert!(change.contains_key("low"), "should have low");
    assert!(change.contains_key("open"), "should have open");
    assert!(change.contains_key("close"), "should have close");
}

#[test]
fn book_changes_ledger_not_found() {
    let source = source();
    let params = object([("ledger_index", JsonValue::Unsigned(999))]);

    let result = do_book_changes(
        &BookChangesRequest {
            params: &params,
            api_version: 2,
            role: RpcRole::Guest,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert!(result.contains_key("error"));
}

#[test]
fn book_changes_response_structure_validated() {
    let source = source();
    let params = object([("ledger_index", JsonValue::String("validated".to_owned()))]);

    let result = do_book_changes(
        &BookChangesRequest {
            params: &params,
            api_version: 2,
            role: RpcRole::Guest,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        result.get("ledger_hash"),
        Some(&JsonValue::String(
            Uint256::from_array([0x22; 32]).to_string()
        ))
    );
    assert_eq!(result.get("ledger_index"), Some(&JsonValue::Unsigned(199)));
    assert_eq!(
        result.get("type"),
        Some(&JsonValue::String("bookChanges".to_owned()))
    );
    assert_eq!(result.get("ledger_time"), Some(&JsonValue::Unsigned(666)));
    assert!(result.contains_key("changes"));
}
