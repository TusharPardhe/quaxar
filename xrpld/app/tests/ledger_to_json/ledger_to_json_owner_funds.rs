use app::{AppLedgerFill, LedgerToJsonContext, LedgerTxEntry, get_json};
use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use ledger::{Ledger, LedgerFillOptions, LedgerHeader};
use protocol::{
    AccountID, IOUAmount, Issue, JsonValue, STAmount, STTx, TxType, currency_from_string,
    get_field_by_symbol,
};
use std::collections::BTreeMap;

fn account(hex: &str) -> AccountID {
    AccountID::from_hex(hex).expect("account hex should parse")
}

fn hash(fill: u8) -> SHAMapHash {
    SHAMapHash::new(Uint256::from_array([fill; 32]))
}

fn ledger() -> Ledger {
    Ledger::new(
        LedgerHeader {
            seq: 1,
            hash: hash(0x11),
            ..LedgerHeader::default()
        },
        false,
    )
}

fn offer_create_tx(sequence: u32, taker_gets: STAmount) -> STTx {
    let source = account("1111111111111111111111111111111111111111");

    STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), source);
        tx.set_field_amount(get_field_by_symbol("sfTakerGets"), taker_gets);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(10, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(12, false),
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
        false
    }

    fn get_close_time_by_seq(&self, _ledger_seq: u32) -> Option<basics::chrono::NetClockTimePoint> {
        None
    }

    fn account_funds(
        &self,
        _ledger: &Ledger,
        owner: AccountID,
        amount: &STAmount,
    ) -> Option<String> {
        assert_eq!(owner, account("1111111111111111111111111111111111111111"));
        assert_eq!(amount.mantissa(), 50);
        Some("900".to_string())
    }
}

fn object(value: JsonValue) -> BTreeMap<String, JsonValue> {
    let JsonValue::Object(object) = value else {
        panic!("json value must be an object");
    };
    object
}

#[test]
fn ledger_to_json_owner_funds_inserts_non_self_funded_offer_balance() {
    let ledger = ledger();
    let tx = offer_create_tx(1, STAmount::new_native(50, false));
    let txs = [LedgerTxEntry {
        txn: &tx,
        meta: None,
    }];

    let rendered = get_json(
        &AppLedgerFill::new(
            &ledger,
            LedgerFillOptions::DUMP_TXRP
                | LedgerFillOptions::EXPAND
                | LedgerFillOptions::OWNER_FUNDS,
        )
        .with_context(&TestContext)
        .with_transactions(&txs),
    )
    .expect("owner funds ledger json should render");
    let rendered = object(rendered);
    let JsonValue::Array(entries) = rendered
        .get("transactions")
        .cloned()
        .expect("transactions should be present")
    else {
        panic!("transactions should be an array");
    };
    let entry = object(entries[0].clone());

    assert_eq!(
        entry.get("owner_funds"),
        Some(&JsonValue::String("900".to_string()))
    );
}

#[test]
fn ledger_to_json_owner_funds_skips_self_funded_offers() {
    let ledger = ledger();
    let source = account("1111111111111111111111111111111111111111");
    let usd = currency_from_string("USD");
    let self_funded_amount = STAmount::from_iou_amount(
        get_field_by_symbol("sfTakerGets"),
        IOUAmount::from_parts(25, 0).expect("IOU amount"),
        Issue::new(usd, source),
    );
    let tx = offer_create_tx(2, self_funded_amount);
    let txs = [LedgerTxEntry {
        txn: &tx,
        meta: None,
    }];

    let rendered = get_json(
        &AppLedgerFill::new(
            &ledger,
            LedgerFillOptions::DUMP_TXRP
                | LedgerFillOptions::EXPAND
                | LedgerFillOptions::OWNER_FUNDS,
        )
        .with_context(&TestContext)
        .with_transactions(&txs),
    )
    .expect("self-funded offer ledger json should render");
    let rendered = object(rendered);
    let JsonValue::Array(entries) = rendered
        .get("transactions")
        .cloned()
        .expect("transactions should be present")
    else {
        panic!("transactions should be an array");
    };
    let entry = object(entries[0].clone());

    assert!(!entry.contains_key("owner_funds"));
}
