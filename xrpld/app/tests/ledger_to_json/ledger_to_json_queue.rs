use app::{AppLedgerFill, LedgerToJsonContext, add_json};
use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use basics::sha_map_hash::SHAMapHash;
use ledger::{Ledger, LedgerFillOptions, LedgerHeader};
use protocol::{
    AccountID, JsonValue, STAmount, STTx, SeqProxy, Ter, TxType, get_field_by_symbol, to_base58,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use tx::{TxConsequences, TxDetails};

fn account(hex: &str) -> AccountID {
    AccountID::from_hex(hex).expect("account hex should parse")
}

fn hash(fill: u8) -> SHAMapHash {
    SHAMapHash::new(Uint256::from_array([fill; 32]))
}

fn ledger(seq: u32, immutable: bool) -> Ledger {
    let mut ledger = Ledger::new(
        LedgerHeader {
            seq,
            hash: hash(0x11),
            close_time: 600_000_000,
            close_time_resolution: 30,
            ..LedgerHeader::default()
        },
        false,
    );
    if immutable {
        ledger.set_immutable(false);
    }
    ledger
}

fn payment_tx(sequence: u32) -> STTx {
    STTx::new(TxType::PAYMENT, |tx| {
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
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    })
}

fn queue_entry() -> TxDetails<Arc<STTx>, AccountID> {
    TxDetails {
        fee_level: 512,
        last_valid: Some(90),
        consequences: TxConsequences::with_potential_spend(10, SeqProxy::sequence(11), 25),
        account: account("1111111111111111111111111111111111111111"),
        seq_proxy: SeqProxy::sequence(11),
        tx: Arc::new(payment_tx(11)),
        retries_remaining: 4,
        preflight_result: Ter::TES_SUCCESS,
        last_result: Some(Ter::TER_RETRY),
    }
}

#[derive(Debug)]
struct TestContext {
    api_version: u32,
    validated: bool,
}

impl LedgerToJsonContext for TestContext {
    fn api_version(&self) -> u32 {
        self.api_version
    }

    fn is_validated(&self, _ledger: &Ledger) -> bool {
        self.validated
    }

    fn get_close_time_by_seq(&self, _ledger_seq: u32) -> Option<NetClockTimePoint> {
        Some(NetClockTimePoint::from(600_000_100))
    }
}

fn object(value: JsonValue) -> BTreeMap<String, JsonValue> {
    let JsonValue::Object(object) = value else {
        panic!("json value must be an object");
    };
    object
}

#[test]
fn ledger_to_json_queue_v2_expanded_merges_tx_shape_into_queue_entry() {
    let ledger = ledger(77, true);
    let context = TestContext {
        api_version: 2,
        validated: true,
    };
    let queue = [queue_entry()];
    let fill = AppLedgerFill::new(
        &ledger,
        LedgerFillOptions::DUMP_QUEUE | LedgerFillOptions::EXPAND,
    )
    .with_context(&context)
    .with_tx_queue(&queue);

    let mut rendered = JsonValue::Null;
    add_json(&mut rendered, &fill).expect("queue json should render");
    let rendered = object(rendered);
    let JsonValue::Array(entries) = rendered
        .get("queue_data")
        .cloned()
        .expect("queue_data should be present")
    else {
        panic!("queue_data should be an array");
    };
    assert_eq!(entries.len(), 1);

    let entry = object(entries[0].clone());
    assert_eq!(
        entry.get("fee_level"),
        Some(&JsonValue::String("512".to_string()))
    );
    assert_eq!(
        entry.get("LastLedgerSequence"),
        Some(&JsonValue::Unsigned(90))
    );
    assert_eq!(entry.get("fee"), Some(&JsonValue::String("10".to_string())));
    assert_eq!(
        entry.get("max_spend_drops"),
        Some(&JsonValue::String("35".to_string()))
    );
    assert_eq!(entry.get("auth_change"), Some(&JsonValue::Bool(false)));
    assert_eq!(
        entry.get("account"),
        Some(&JsonValue::String(to_base58(account(
            "1111111111111111111111111111111111111111"
        ))))
    );
    assert_eq!(entry.get("retries_remaining"), Some(&JsonValue::Signed(4)));
    assert_eq!(
        entry.get("preflight_result"),
        Some(&JsonValue::String("tesSUCCESS".to_string()))
    );
    assert_eq!(
        entry.get("last_result"),
        Some(&JsonValue::String("terRETRY".to_string()))
    );
    assert_eq!(entry.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(entry.get("ledger_index"), Some(&JsonValue::Unsigned(77)));
    assert_eq!(
        entry.get("close_time_iso"),
        Some(&JsonValue::String("2019-01-05T10:41:40Z".to_string()))
    );
    assert_eq!(
        entry.get("ledger_hash"),
        Some(&JsonValue::String(ledger.header().hash.to_string()))
    );

    let tx_json = object(
        entry
            .get("tx_json")
            .cloned()
            .expect("queue v2 should include tx_json"),
    );
    assert!(tx_json.contains_key("DeliverMax"));
    assert!(!tx_json.contains_key("Amount"));
}

#[test]
fn ledger_to_json_queue_v2_non_expanded_uses_hash_field() {
    let ledger = ledger(77, true);
    let queue = [queue_entry()];
    let fill = AppLedgerFill::new(&ledger, LedgerFillOptions::DUMP_QUEUE)
        .with_api_version(2)
        .with_tx_queue(&queue);

    let mut rendered = JsonValue::Null;
    add_json(&mut rendered, &fill).expect("queue json should render");
    let rendered = object(rendered);
    let JsonValue::Array(entries) = rendered
        .get("queue_data")
        .cloned()
        .expect("queue_data should be present")
    else {
        panic!("queue_data should be an array");
    };

    let entry = object(entries[0].clone());
    assert_eq!(
        entry.get("hash"),
        Some(&JsonValue::String(
            queue[0].tx.get_transaction_id().to_string()
        ))
    );
    assert!(!entry.contains_key("tx"));
}

#[test]
fn ledger_to_json_queue_legacy_non_expanded_nests_tx_under_tx() {
    let ledger = ledger(77, true);
    let queue = [queue_entry()];
    let fill = AppLedgerFill::new(&ledger, LedgerFillOptions::DUMP_QUEUE)
        .with_api_version(1)
        .with_tx_queue(&queue);

    let mut rendered = JsonValue::Null;
    add_json(&mut rendered, &fill).expect("legacy queue json should render");
    let rendered = object(rendered);
    let JsonValue::Array(entries) = rendered
        .get("queue_data")
        .cloned()
        .expect("queue_data should be present")
    else {
        panic!("queue_data should be an array");
    };

    let entry = object(entries[0].clone());
    assert_eq!(
        entry.get("tx"),
        Some(&JsonValue::String(
            queue[0].tx.get_transaction_id().to_string()
        ))
    );
    assert!(!entry.contains_key("validated"));
    assert!(!entry.contains_key("tx_json"));
}
