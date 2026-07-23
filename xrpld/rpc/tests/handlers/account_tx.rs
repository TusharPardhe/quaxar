//! Tests for the account tx RPC handler.

use std::{collections::BTreeMap, sync::Arc};

use basics::{base_uint::Uint256, chrono::NetClockTimePoint};
use protocol::{
    AccountID, JsonValue, MPTAmount, MPTIssue, STAmount, STArray, STObject, STTx, TxMeta, TxType,
    get_field_by_symbol,
};
use rpc::{
    AccountTxLedgerRange, AccountTxMarker, AccountTxPage, AccountTxQuery, AccountTxSource,
    LedgerLookupLedger, LedgerLookupSource, RpcErrorCode, RpcRole, do_account_tx,
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

fn payment_tx() -> STTx {
    STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(1));
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(2));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 5);
    })
}

fn payment_meta(tx_id: Uint256) -> TxMeta {
    let mut object = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    object.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    object.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 3);
    object.set_field_amount(
        get_field_by_symbol("sfDeliveredAmount"),
        STAmount::new_native(1_000_000, false),
    );
    object.set_field_array(
        get_field_by_symbol("sfAffectedNodes"),
        STArray::new(get_field_by_symbol("sfAffectedNodes")),
    );
    TxMeta::from_stobject(tx_id, 22, object)
}

#[derive(Clone)]
struct FakeSource {
    current: Option<LedgerLookupLedger>,
    closed: Option<LedgerLookupLedger>,
    validated: Option<LedgerLookupLedger>,
    page: AccountTxPage,
    close_times: BTreeMap<u32, NetClockTimePoint>,
}

impl LedgerLookupSource for FakeSource {
    fn get_ledger_by_hash(&self, hash: Uint256) -> Option<LedgerLookupLedger> {
        [self.current, self.closed, self.validated]
            .into_iter()
            .flatten()
            .find(|ledger| ledger.hash == hash)
    }

    fn get_ledger_by_seq(&self, seq: u32) -> Option<LedgerLookupLedger> {
        [self.current, self.closed, self.validated]
            .into_iter()
            .flatten()
            .find(|ledger| ledger.seq == seq)
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
        self.validated.map(|ledger| ledger.seq).unwrap_or_default()
    }

    fn get_validated_ledger_age(&self) -> std::time::Duration {
        std::time::Duration::from_secs(5)
    }

    fn is_validated(&self, ledger: &LedgerLookupLedger) -> bool {
        self.validated == Some(*ledger)
    }
}

impl AccountTxSource for FakeSource {
    fn validated_range(&self) -> Option<AccountTxLedgerRange> {
        Some(AccountTxLedgerRange { min: 20, max: 30 })
    }

    fn page(&self, query: &AccountTxQuery) -> Result<AccountTxPage, rpc::RpcStatus> {
        assert_eq!(
            query.ledger_range,
            AccountTxLedgerRange { min: 22, max: 22 }
        );
        Ok(self.page.clone())
    }

    fn get_close_time_by_seq(&self, seq: u32) -> Option<NetClockTimePoint> {
        self.close_times.get(&seq).copied()
    }
}

fn source() -> FakeSource {
    let validated = LedgerLookupLedger {
        hash: Uint256::from_array([0x22; 32]),
        seq: 22,
        open: false,
    };
    let txn = Arc::new(payment_tx());
    let meta = payment_meta(txn.get_transaction_id());
    FakeSource {
        current: Some(validated),
        closed: Some(validated),
        validated: Some(validated),
        page: AccountTxPage {
            ledger_range: AccountTxLedgerRange { min: 22, max: 22 },
            limit: 25,
            marker: Some(AccountTxMarker { ledger: 22, seq: 8 }),
            transactions: vec![rpc::TxRecord {
                txn,
                meta: Some(meta),
                ledger_index: 22,
                close_time: Some(NetClockTimePoint::new(600_000_100)),
                ledger_hash: Some(validated.hash),
                validated: true,
                txn_index: Some(3),
                network_id: Some(0),
            }],
        },
        close_times: BTreeMap::from([(22, NetClockTimePoint::new(600_000_100))]),
    }
}

#[test]
fn account_tx_preserves_persisted_mpt_delivered_amount_in_json_and_binary() {
    let mpt_issue = MPTIssue::new(protocol::make_mpt_id(1, account(3)));
    let tx = Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(1));
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(2));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::from_mpt_amount(
                get_field_by_symbol("sfAmount"),
                MPTAmount::from_value(1_000),
                mpt_issue,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 5);
    }));
    let delivered = STAmount::from_mpt_amount(
        get_field_by_symbol("sfDeliveredAmount"),
        MPTAmount::from_value(800),
        mpt_issue,
    );
    let mut on_meta = payment_meta(tx.get_transaction_id());
    on_meta.set_delivered_amount(Some(delivered));
    let mut off_meta = payment_meta(tx.get_transaction_id());
    off_meta.set_delivered_amount(None);
    let validated = LedgerLookupLedger {
        hash: Uint256::from_array([0x22; 32]),
        seq: 22,
        open: false,
    };
    let source = FakeSource {
        current: Some(validated),
        closed: Some(validated),
        validated: Some(validated),
        page: AccountTxPage {
            ledger_range: AccountTxLedgerRange { min: 22, max: 22 },
            limit: 25,
            marker: None,
            transactions: vec![
                rpc::TxRecord {
                    txn: Arc::clone(&tx),
                    meta: Some(on_meta.clone()),
                    ledger_index: 22,
                    close_time: Some(NetClockTimePoint::new(600_000_100)),
                    ledger_hash: Some(validated.hash),
                    validated: true,
                    txn_index: Some(3),
                    network_id: Some(0),
                },
                rpc::TxRecord {
                    txn: Arc::clone(&tx),
                    meta: Some(off_meta.clone()),
                    ledger_index: 22,
                    close_time: None,
                    ledger_hash: Some(validated.hash),
                    validated: true,
                    txn_index: Some(4),
                    network_id: Some(0),
                },
            ],
        },
        close_times: BTreeMap::from([(22, NetClockTimePoint::new(600_000_100))]),
    };
    let json_params = object([
        (
            "account",
            JsonValue::String(protocol::to_base58(account(1))),
        ),
        ("ledger_index", JsonValue::String("validated".to_owned())),
    ]);
    let json = do_account_tx(&json_params, RpcRole::Admin, 2, &source);
    let JsonValue::Array(json_txs) = json.get("transactions").expect("transactions") else {
        panic!("transactions must be an array");
    };
    let JsonValue::Object(on_entry) = &json_txs[0] else {
        panic!("on entry must be an object");
    };
    let JsonValue::Object(on_meta_json) = on_entry.get("meta").expect("on metadata") else {
        panic!("on metadata must be an object");
    };
    assert_eq!(
        on_meta_json.get("DeliveredAmount"),
        on_meta_json.get("delivered_amount")
    );
    let JsonValue::Object(off_entry) = &json_txs[1] else {
        panic!("off entry must be an object");
    };
    let JsonValue::Object(off_meta_json) = off_entry.get("meta").expect("off metadata") else {
        panic!("off metadata must be an object");
    };
    assert_eq!(
        off_meta_json.get("delivered_amount"),
        Some(&JsonValue::String("unavailable".to_owned()))
    );
    assert!(!off_meta_json.contains_key("DeliveredAmount"));

    let binary_params = object([
        (
            "account",
            JsonValue::String(protocol::to_base58(account(1))),
        ),
        ("ledger_index", JsonValue::String("validated".to_owned())),
        ("binary", JsonValue::Bool(true)),
    ]);
    let binary = do_account_tx(&binary_params, RpcRole::Admin, 2, &source);
    let JsonValue::Array(binary_txs) = binary.get("transactions").expect("transactions") else {
        panic!("transactions must be an array");
    };
    let JsonValue::Object(on_binary) = &binary_txs[0] else {
        panic!("on binary entry must be an object");
    };
    assert_eq!(
        on_binary.get("meta_blob"),
        Some(&JsonValue::String(basics::str_hex::str_hex(
            on_meta.get_as_object().get_serializer().data()
        )))
    );
    let JsonValue::Object(off_binary) = &binary_txs[1] else {
        panic!("off binary entry must be an object");
    };
    assert_eq!(
        off_binary.get("meta_blob"),
        Some(&JsonValue::String(basics::str_hex::str_hex(
            off_meta.get_as_object().get_serializer().data()
        )))
    );
}

#[test]
fn account_tx_v2_renders_tx_json_marker_and_ledger_metadata() {
    let source = source();
    let params = object([
        (
            "account",
            JsonValue::String(protocol::to_base58(account(1))),
        ),
        ("ledger_index", JsonValue::String("validated".to_owned())),
        ("limit", JsonValue::Unsigned(25)),
        ("forward", JsonValue::Bool(true)),
    ]);

    let result = do_account_tx(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(result.get("limit"), Some(&JsonValue::Unsigned(25)));
    assert_eq!(
        result.get("ledger_index_min"),
        Some(&JsonValue::Unsigned(22))
    );
    assert_eq!(
        result.get("ledger_index_max"),
        Some(&JsonValue::Unsigned(22))
    );
    assert!(result.contains_key("marker"));

    let JsonValue::Array(transactions) = result.get("transactions").expect("transactions") else {
        panic!("transactions must be an array");
    };
    let JsonValue::Object(first) = &transactions[0] else {
        panic!("first transaction must be an object");
    };
    assert!(first.contains_key("tx_json"));
    assert!(first.contains_key("hash"));
    assert!(first.contains_key("ledger_hash"));
    assert!(first.contains_key("close_time_iso"));
    assert!(first.contains_key("meta"));
}

#[test]
fn account_tx_rejects_non_boolean_binary_in_api_v2() {
    let source = source();
    let params = object([
        (
            "account",
            JsonValue::String(protocol::to_base58(account(1))),
        ),
        ("ledger_index", JsonValue::String("validated".to_owned())),
        ("binary", JsonValue::String("yes".to_owned())),
    ]);

    let result = do_account_tx(&params, RpcRole::User, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::InvalidParams.token().to_owned()
        ))
    );
}

#[test]
fn account_tx_v2_rejects_min_below_validated_range() {
    let source = source();
    let params = object([
        (
            "account",
            JsonValue::String(protocol::to_base58(account(1))),
        ),
        ("ledger_index_min", JsonValue::Unsigned(19)),
        ("ledger_index_max", JsonValue::Unsigned(22)),
    ]);

    let result = do_account_tx(&params, RpcRole::User, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::LedgerIndexMalformed.token().to_owned()
        ))
    );
}

#[test]
fn account_tx_v2_rejects_max_above_validated_range() {
    let source = source();
    let params = object([
        (
            "account",
            JsonValue::String(protocol::to_base58(account(1))),
        ),
        ("ledger_index_min", JsonValue::Unsigned(22)),
        ("ledger_index_max", JsonValue::Unsigned(31)),
    ]);

    let result = do_account_tx(&params, RpcRole::User, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::LedgerIndexMalformed.token().to_owned()
        ))
    );
}

#[test]
fn account_tx_v1_reports_legacy_invalid_index_range_token() {
    let source = source();
    let params = object([
        (
            "account",
            JsonValue::String(protocol::to_base58(account(1))),
        ),
        ("ledger_index_min", JsonValue::Unsigned(30)),
        ("ledger_index_max", JsonValue::Unsigned(20)),
    ]);

    let result = do_account_tx(&params, RpcRole::User, 1, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::LedgerIndexesInvalid.token().to_owned()
        ))
    );
}

#[test]
fn account_tx_missing_account_field() {
    let source = source();
    let params = object([("ledger_index", JsonValue::String("validated".to_owned()))]);

    let result = do_account_tx(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert!(result.contains_key("error"));
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::InvalidParams.token().to_owned()
        ))
    );
}

#[test]
fn account_tx_invalid_account_types() {
    let source = source();

    for param in [
        JsonValue::Unsigned(1),
        JsonValue::Bool(true),
        JsonValue::Null,
        JsonValue::Array(vec![]),
        JsonValue::Object(Default::default()),
    ] {
        let params = object([("account", param)]);
        let result = do_account_tx(&params, RpcRole::Admin, 2, &source);
        let JsonValue::Object(result) = result else {
            panic!("result must be an object");
        };
        assert!(
            result.contains_key("error"),
            "invalid account type should produce error"
        );
    }
}

#[test]
fn account_tx_malformed_account_string() {
    let source = source();
    let params = object([("account", JsonValue::String("notAnAccount".to_owned()))]);

    let result = do_account_tx(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::ActMalformed.token().to_owned()
        ))
    );
}

#[test]
fn account_tx_v2_transaction_fields_complete() {
    let source = source();
    let params = object([
        (
            "account",
            JsonValue::String(protocol::to_base58(account(1))),
        ),
        ("ledger_index", JsonValue::String("validated".to_owned())),
    ]);

    let result = do_account_tx(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None);
    assert_eq!(
        result.get("account"),
        Some(&JsonValue::String(protocol::to_base58(account(1))))
    );

    let JsonValue::Array(transactions) = result.get("transactions").expect("transactions") else {
        panic!("transactions must be an array");
    };
    assert_eq!(transactions.len(), 1);

    let JsonValue::Object(entry) = &transactions[0] else {
        panic!("entry must be an object");
    };
    // v2 should have tx_json, not tx
    assert!(entry.contains_key("tx_json"), "v2 should have tx_json");
    assert!(entry.contains_key("hash"), "should have hash");
    assert!(
        entry.contains_key("ledger_index"),
        "should have ledger_index"
    );
    assert!(entry.contains_key("ledger_hash"), "should have ledger_hash");
    assert_eq!(entry.get("validated"), Some(&JsonValue::Bool(true)));

    // Check tx_json has expected fields
    let JsonValue::Object(tx_json) = entry.get("tx_json").expect("tx_json") else {
        panic!("tx_json must be an object");
    };
    assert_eq!(
        tx_json.get("TransactionType"),
        Some(&JsonValue::String("Payment".to_owned()))
    );
    assert!(tx_json.contains_key("Account"));
    assert!(tx_json.contains_key("Sequence"));
}

#[test]
fn account_tx_v1_uses_flat_tx_fields_not_tx_json() {
    let source = source();
    let params = object([
        (
            "account",
            JsonValue::String(protocol::to_base58(account(1))),
        ),
        ("ledger_index", JsonValue::String("validated".to_owned())),
    ]);

    let result = do_account_tx(&params, RpcRole::Admin, 1, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None);

    let JsonValue::Array(transactions) = result.get("transactions").expect("transactions") else {
        panic!("transactions must be an array");
    };
    assert_eq!(transactions.len(), 1);

    let JsonValue::Object(entry) = &transactions[0] else {
        panic!("entry must be an object");
    };
    // v1 flattens tx fields at top level (no "tx_json" wrapper)
    assert!(!entry.contains_key("tx_json"), "v1 should NOT have tx_json");
    assert!(entry.contains_key("meta"), "should have meta");
    assert_eq!(entry.get("validated"), Some(&JsonValue::Bool(true)));
    // v1 has TransactionType at top level
    assert!(entry.contains_key("TransactionType") || entry.contains_key("hash"));
}

#[test]
fn account_tx_marker_structure() {
    let source = source();
    let params = object([
        (
            "account",
            JsonValue::String(protocol::to_base58(account(1))),
        ),
        ("ledger_index", JsonValue::String("validated".to_owned())),
    ]);

    let result = do_account_tx(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    // Marker should be present (our fake source returns one)
    let JsonValue::Object(marker) = result.get("marker").expect("marker") else {
        panic!("marker must be an object");
    };
    assert_eq!(marker.get("ledger"), Some(&JsonValue::Unsigned(22)));
    assert_eq!(marker.get("seq"), Some(&JsonValue::Unsigned(8)));
}

#[test]
fn account_tx_rejects_non_boolean_forward_in_v2() {
    let source = source();
    let params = object([
        (
            "account",
            JsonValue::String(protocol::to_base58(account(1))),
        ),
        ("ledger_index", JsonValue::String("validated".to_owned())),
        ("forward", JsonValue::String("yes".to_owned())),
    ]);

    let result = do_account_tx(&params, RpcRole::User, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::InvalidParams.token().to_owned()
        ))
    );
}
