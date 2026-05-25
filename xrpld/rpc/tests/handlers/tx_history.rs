//! Tests for the tx history RPC handler.

mod ledger_lookup {
    pub use rpc::RpcRole;
}

use std::{cell::RefCell, collections::BTreeMap, sync::Arc};

use app::{TransStatus, Transaction};
use protocol::{AccountID, JsonValue, STAmount, STTx, TxType, get_field_by_symbol};
use rpc::TxHistoryRow;
use rpc::{TxHistoryRequest, TxHistorySource, do_tx_history};

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
    })
}

fn offer_tx() -> STTx {
    STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(3));
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(10, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfTakerGets"),
            STAmount::new_native(20, false),
        );
    })
}

fn history_row(tx: STTx, ledger_seq: u32) -> TxHistoryRow {
    let mut transaction = Transaction::new(Arc::new(tx));
    transaction.set_status_with_ledger(TransStatus::COMMITTED, ledger_seq, None, None);
    TxHistoryRow {
        transaction: Arc::new(transaction),
    }
}

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[derive(Debug, Clone)]
struct FakeTxHistorySource {
    enabled: bool,
    requested_starts: RefCell<Vec<u32>>,
    txs_by_start: BTreeMap<u32, Vec<TxHistoryRow>>,
}

impl FakeTxHistorySource {
    fn new(enabled: bool, txs_by_start: BTreeMap<u32, Vec<TxHistoryRow>>) -> Self {
        Self {
            enabled,
            requested_starts: RefCell::new(Vec::new()),
            txs_by_start,
        }
    }

    fn requested_starts(&self) -> Vec<u32> {
        self.requested_starts.borrow().clone()
    }
}

impl TxHistorySource for FakeTxHistorySource {
    type Row = TxHistoryRow;

    fn tx_tables_enabled(&self) -> bool {
        self.enabled
    }

    fn get_tx_history(&self, start_index: u32) -> Vec<Self::Row> {
        self.requested_starts.borrow_mut().push(start_index);
        self.txs_by_start
            .get(&start_index)
            .cloned()
            .unwrap_or_default()
    }
}

#[test]
fn tx_history_requires_enabled_tables_and_start() {
    let source = FakeTxHistorySource::new(false, BTreeMap::new());
    let request = TxHistoryRequest {
        params: &object([]),
        role: ledger_lookup::RpcRole::User,
        api_version: 1,
    };

    let result = do_tx_history(&request, &source);
    assert_eq!(
        result,
        JsonValue::Object(BTreeMap::from([
            (
                "error".to_owned(),
                JsonValue::String("notEnabled".to_owned())
            ),
            ("error_code".to_owned(), JsonValue::Signed(12)),
            (
                "error_message".to_owned(),
                JsonValue::String("Not enabled in configuration.".to_owned())
            ),
        ]))
    );
    assert!(source.requested_starts().is_empty());

    let source = FakeTxHistorySource::new(true, BTreeMap::new());
    let request = TxHistoryRequest {
        params: &object([]),
        role: ledger_lookup::RpcRole::User,
        api_version: 1,
    };

    let result = do_tx_history(&request, &source);
    assert_eq!(
        result,
        JsonValue::Object(BTreeMap::from([
            (
                "error".to_owned(),
                JsonValue::String("invalidParams".to_owned())
            ),
            ("error_code".to_owned(), JsonValue::Signed(31)),
            (
                "error_message".to_owned(),
                JsonValue::String("Invalid parameters.".to_owned())
            ),
        ]))
    );
    assert!(source.requested_starts().is_empty());
}

#[test]
fn tx_history_allows_direct_v2_calls_and_still_shapes_deliver_max() {
    let source = FakeTxHistorySource::new(
        true,
        BTreeMap::from([(
            0,
            vec![history_row(payment_tx(), 44), history_row(offer_tx(), 45)],
        )]),
    );

    let request = TxHistoryRequest {
        params: &object([("start", JsonValue::Unsigned(0))]),
        role: ledger_lookup::RpcRole::User,
        api_version: 2,
    };

    let result = do_tx_history(&request, &source);
    let JsonValue::Object(result_object) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result_object.get("index"), Some(&JsonValue::Unsigned(0)));
    let JsonValue::Array(txs) = result_object.get("txs").expect("txs array") else {
        panic!("txs must be an array");
    };
    let JsonValue::Object(payment) = &txs[0] else {
        panic!("payment tx must be an object");
    };
    assert!(!payment.contains_key("Amount"));
    assert!(payment.contains_key("DeliverMax"));

    let JsonValue::Object(offer) = &txs[1] else {
        panic!("offer tx must be an object");
    };
    assert!(!offer.contains_key("DeliverMax"));
    assert_eq!(source.requested_starts(), vec![0]);
}

#[test]
fn tx_history_enforces_role_gate_above_10000() {
    let source = FakeTxHistorySource::new(
        true,
        BTreeMap::from([(
            10_000,
            vec![history_row(payment_tx(), 44), history_row(offer_tx(), 45)],
        )]),
    );

    let request = TxHistoryRequest {
        params: &object([("start", JsonValue::Unsigned(10_001))]),
        role: ledger_lookup::RpcRole::User,
        api_version: 1,
    };

    let result = do_tx_history(&request, &source);
    assert_eq!(
        result,
        JsonValue::Object(BTreeMap::from([
            (
                "error".to_owned(),
                JsonValue::String("noPermission".to_owned())
            ),
            ("error_code".to_owned(), JsonValue::Signed(6)),
            (
                "error_message".to_owned(),
                JsonValue::String("You don't have permission for this command.".to_owned())
            ),
        ]))
    );
    assert!(source.requested_starts().is_empty());

    let request = TxHistoryRequest {
        params: &object([("start", JsonValue::Unsigned(10_000))]),
        role: ledger_lookup::RpcRole::Admin,
        api_version: 1,
    };

    let result = do_tx_history(&request, &source);
    let JsonValue::Object(result_object) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result_object.get("index"),
        Some(&JsonValue::Unsigned(10_000))
    );
    let JsonValue::Array(txs) = result_object.get("txs").expect("txs array") else {
        panic!("txs must be an array");
    };
    assert_eq!(txs.len(), 2);
    assert_eq!(source.requested_starts(), vec![10_000]);
}

#[test]
fn tx_history_inserts_deliver_max() {
    let source = FakeTxHistorySource::new(
        true,
        BTreeMap::from([(
            0,
            vec![history_row(payment_tx(), 44), history_row(offer_tx(), 45)],
        )]),
    );

    let request_v1 = TxHistoryRequest {
        params: &object([("start", JsonValue::Unsigned(0))]),
        role: ledger_lookup::RpcRole::Admin,
        api_version: 1,
    };
    let result = do_tx_history(&request_v1, &source);
    let JsonValue::Object(result_object) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Array(txs) = result_object.get("txs").expect("txs array") else {
        panic!("txs must be an array");
    };

    let JsonValue::Object(payment) = &txs[0] else {
        panic!("payment tx must be an object");
    };
    assert_eq!(payment.get("inLedger"), Some(&JsonValue::Unsigned(44)));
    assert_eq!(payment.get("ledger_index"), Some(&JsonValue::Unsigned(44)));
    assert!(payment.contains_key("Amount"));
    assert_eq!(payment.get("DeliverMax"), payment.get("Amount"));

    let JsonValue::Object(offer) = &txs[1] else {
        panic!("offer tx must be an object");
    };
    assert_eq!(offer.get("inLedger"), Some(&JsonValue::Unsigned(45)));
    assert_eq!(offer.get("ledger_index"), Some(&JsonValue::Unsigned(45)));
    assert!(!offer.contains_key("DeliverMax"));
}
