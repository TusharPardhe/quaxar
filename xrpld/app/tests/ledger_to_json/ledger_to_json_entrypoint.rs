use app::{AppLedgerFill, LedgerToJsonContext, add_json, copy_from, get_json};
use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use basics::sha_map_hash::SHAMapHash;
use ledger::{Ledger, LedgerFillOptions, LedgerHeader};
use protocol::{AccountID, JsonValue, STAmount, STTx, SeqProxy, Ter, TxType, get_field_by_symbol};
use std::collections::BTreeMap;
use std::sync::Arc;
use tx::{TxConsequences, TxDetails};

fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn hash(fill: u8) -> SHAMapHash {
    SHAMapHash::new(Uint256::from_array([fill; 32]))
}

fn ledger() -> Ledger {
    let mut ledger = Ledger::new(
        LedgerHeader {
            seq: 91,
            hash: hash(0x91),
            parent_hash: hash(0x92),
            ..LedgerHeader::default()
        },
        false,
    );
    ledger.set_immutable(false);
    ledger
}

fn payment_tx(sequence: u32) -> STTx {
    STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(0x11));
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(0x22));
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

#[derive(Debug)]
struct TestContext;

impl LedgerToJsonContext for TestContext {
    fn api_version(&self) -> u32 {
        2
    }

    fn is_validated(&self, _ledger: &Ledger) -> bool {
        true
    }

    fn get_close_time_by_seq(&self, ledger_seq: u32) -> Option<NetClockTimePoint> {
        Some(NetClockTimePoint::from(
            600_000_000 + u64::from(ledger_seq) as u32,
        ))
    }
}

fn queue_entry() -> TxDetails<Arc<STTx>, AccountID> {
    TxDetails {
        fee_level: 12,
        last_valid: Some(99),
        consequences: TxConsequences::new(10, SeqProxy::sequence(7)),
        account: account(0x11),
        seq_proxy: SeqProxy::sequence(7),
        tx: Arc::new(payment_tx(7)),
        retries_remaining: 3,
        preflight_result: Ter::TES_SUCCESS,
        last_result: None,
    }
}

fn object(value: JsonValue) -> BTreeMap<String, JsonValue> {
    let JsonValue::Object(object) = value else {
        panic!("json value must be an object");
    };
    object
}

#[test]
fn ledger_to_json_add_json_wraps_ledger_and_queue_data() {
    let ledger = ledger();
    let queue = [queue_entry()];
    let fill = AppLedgerFill::new(
        &ledger,
        LedgerFillOptions::FULL | LedgerFillOptions::DUMP_QUEUE | LedgerFillOptions::EXPAND,
    )
    .with_context(&TestContext)
    .with_tx_queue(&queue);

    let mut rendered = JsonValue::Null;
    add_json(&mut rendered, &fill).expect("add_json should render");
    let rendered = object(rendered);

    assert!(rendered.contains_key("ledger"));
    assert!(rendered.contains_key("queue_data"));

    let standalone = get_json(&fill).expect("get_json should render");
    let standalone = object(standalone);
    assert!(!standalone.contains_key("queue_data"));
}

#[test]
fn ledger_to_json_copy_from_merges_object_members() {
    let mut target = JsonValue::Object(BTreeMap::from([(
        "ledger_index".to_string(),
        JsonValue::Unsigned(91),
    )]));
    let source = JsonValue::Object(BTreeMap::from([
        ("validated".to_string(), JsonValue::Bool(true)),
        (
            "close_time_iso".to_string(),
            JsonValue::String("x".to_string()),
        ),
    ]));

    copy_from(&mut target, &source);

    let target = object(target);
    assert_eq!(target.get("ledger_index"), Some(&JsonValue::Unsigned(91)));
    assert_eq!(target.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        target.get("close_time_iso"),
        Some(&JsonValue::String("x".to_string()))
    );
}
