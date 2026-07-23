use app::{AppLedgerFill, LedgerToJsonContext, LedgerTxEntry, get_json, get_json_with_family};
use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use basics::sha_map_hash::SHAMapHash;
use basics::str_hex::str_hex;
use basics::tagged_cache::MonotonicClock;
use ledger::{Ledger, LedgerFillOptions, LedgerHeader};
use protocol::{
    AccountID, JsonValue, LedgerEntryType, MPTID, STAmount, STArray, STObject, STTx, Serializer,
    TxMeta, TxType, get_field_by_symbol, make_mpt_id,
};
use shamap::family::{NullFullBelowCache, NullMissingNodeReporter, NullNodeFetcher, SHAMapFamily};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;
use shamap::tree_node_cache::TreeNodeCache;
use std::collections::BTreeMap;
use std::sync::Arc;

fn account(hex: &str) -> AccountID {
    AccountID::from_hex(hex).expect("account hex should parse")
}

fn hash(fill: u8) -> SHAMapHash {
    SHAMapHash::new(Uint256::from_array([fill; 32]))
}

fn base_ledger(seq: u32, immutable: bool) -> Ledger {
    let mut ledger = Ledger::new(
        LedgerHeader {
            seq,
            hash: hash(0xAA),
            parent_hash: hash(0xAB),
            tx_hash: hash(0xAC),
            account_hash: hash(0xAD),
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

fn account_delete_tx(sequence: u32) -> STTx {
    STTx::new(TxType::ACCOUNT_DELETE, |tx| {
        tx.set_account_id(
            get_field_by_symbol("sfAccount"),
            account("1111111111111111111111111111111111111111"),
        );
        tx.set_account_id(
            get_field_by_symbol("sfDestination"),
            account("2222222222222222222222222222222222222222"),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    })
}

fn mpt_issuance_create_tx(sequence: u32, issuer: AccountID) -> STTx {
    STTx::new(TxType::MPTOKEN_ISSUANCE_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), issuer);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    })
}

fn payment_meta(tx_id: Uint256, ledger_seq: u32, delivered_amount: Option<STAmount>) -> TxMeta {
    let mut object = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    object.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    object.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 2);
    object.set_field_array(
        get_field_by_symbol("sfAffectedNodes"),
        STArray::new(get_field_by_symbol("sfAffectedNodes")),
    );
    if let Some(delivered_amount) = delivered_amount {
        object.set_field_amount(get_field_by_symbol("sfDeliveredAmount"), delivered_amount);
    }
    TxMeta::from_stobject(tx_id, ledger_seq, object)
}

fn mpt_meta(tx_id: Uint256, ledger_seq: u32, sequence: u32, issuer: AccountID) -> (TxMeta, MPTID) {
    let issuance_id = make_mpt_id(sequence, issuer);

    let mut new_fields = STObject::new(get_field_by_symbol("sfNewFields"));
    new_fields.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    new_fields.set_account_id(get_field_by_symbol("sfIssuer"), issuer);

    let mut created = STObject::new(get_field_by_symbol("sfCreatedNode"));
    created.set_field_h256(
        get_field_by_symbol("sfLedgerIndex"),
        Uint256::from_array([0x55; 32]),
    );
    created.set_field_u16(
        get_field_by_symbol("sfLedgerEntryType"),
        LedgerEntryType::MPTokenIssuance.code(),
    );
    created.set_field_object(get_field_by_symbol("sfNewFields"), new_fields);

    let mut affected = STArray::new(get_field_by_symbol("sfAffectedNodes"));
    affected.push_back(created);

    let mut object = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    object.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    object.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 3);
    object.set_field_array(get_field_by_symbol("sfAffectedNodes"), affected);
    (
        TxMeta::from_stobject(tx_id, ledger_seq, object),
        issuance_id,
    )
}

#[derive(Debug)]
struct TestContext {
    api_version: u32,
    validated: bool,
    close_time: Option<NetClockTimePoint>,
}

impl LedgerToJsonContext for TestContext {
    fn api_version(&self) -> u32 {
        self.api_version
    }

    fn is_validated(&self, _ledger: &Ledger) -> bool {
        self.validated
    }

    fn get_close_time_by_seq(&self, _ledger_seq: u32) -> Option<NetClockTimePoint> {
        self.close_time
    }
}

fn object(value: JsonValue) -> BTreeMap<String, JsonValue> {
    let JsonValue::Object(object) = value else {
        panic!("json value must be an object");
    };
    object
}

fn array(value: &JsonValue) -> &[JsonValue] {
    let JsonValue::Array(values) = value else {
        panic!("json value must be an array");
    };
    values
}

#[test]
fn ledger_to_json_tx_family_traverses_transaction_md_leaves() {
    let tx = payment_tx(7);
    let meta = payment_meta(tx.get_transaction_id(), 44, None);
    let mut payload = Serializer::new(0);
    payload.add_vl(tx.get_serializer().data());
    payload.add_vl(meta.get_as_object().get_serializer().data());

    let mut tx_tree = MutableTree::new(44);
    tx_tree
        .add_item(
            SHAMapNodeType::TransactionMd,
            SHAMapItem::new(tx.get_transaction_id(), payload.data().to_vec()),
        )
        .expect("TransactionMd leaf should insert");
    let tx_root = tx_tree.root();
    let mut ledger = Ledger::from_maps(
        LedgerHeader {
            seq: 44,
            hash: hash(0xAA),
            parent_hash: hash(0xAB),
            tx_hash: tx_root.get_hash(),
            ..LedgerHeader::default()
        },
        SyncTree::new_with_type(SHAMapType::State, false, 44),
        SyncTree::from_root_with_type(
            tx_root,
            SHAMapType::Transaction,
            true,
            44,
            SyncState::Immutable,
        ),
    );
    ledger.set_immutable(true);

    let family = SHAMapFamily::new(
        Arc::new(TreeNodeCache::new(
            "ledger-json-transaction-md",
            8,
            time::Duration::seconds(1),
            MonotonicClock::default(),
        )),
        NullFullBelowCache::new(0),
        NullNodeFetcher,
        NullMissingNodeReporter,
    );
    let rendered = get_json_with_family(
        &AppLedgerFill::new(&ledger, LedgerFillOptions::DUMP_TXRP),
        &family,
    )
    .expect("family-aware ledger rendering should succeed");
    let rendered = object(rendered);
    assert_eq!(
        array(
            rendered
                .get("transactions")
                .expect("transactions should be present")
        ),
        &[JsonValue::String(tx.get_transaction_id().to_string())]
    );
}

#[test]
fn ledger_to_json_tx_non_expanded_returns_hash() {
    let ledger = base_ledger(44, false);
    let tx = payment_tx(7);
    let txs = [LedgerTxEntry {
        txn: &tx,
        meta: None,
    }];

    let rendered = get_json(
        &AppLedgerFill::new(&ledger, LedgerFillOptions::DUMP_TXRP)
            .with_transactions(&txs)
            .with_api_version(2),
    )
    .expect("ledger json should render");
    let rendered = object(rendered);
    let txs = array(
        rendered
            .get("transactions")
            .expect("transactions should be present"),
    );

    assert_eq!(
        txs,
        &[JsonValue::String(tx.get_transaction_id().to_string())]
    );
}

#[test]
fn ledger_to_json_tx_binary_uses_legacy_and_v2_meta_field_names() {
    let ledger = base_ledger(44, false);
    let tx = payment_tx(8);
    let meta = payment_meta(tx.get_transaction_id(), 44, None);
    let txs = [LedgerTxEntry {
        txn: &tx,
        meta: Some(&meta),
    }];

    let legacy = get_json(
        &AppLedgerFill::new(
            &ledger,
            LedgerFillOptions::DUMP_TXRP | LedgerFillOptions::EXPAND | LedgerFillOptions::BINARY,
        )
        .with_transactions(&txs)
        .with_api_version(1),
    )
    .expect("legacy binary ledger json should render");
    let legacy = object(legacy);
    let legacy_entry = object(array(legacy.get("transactions").expect("transactions"))[0].clone());
    assert_eq!(
        legacy_entry.get("tx_blob"),
        Some(&JsonValue::String(str_hex(tx.get_serializer().data())))
    );
    assert_eq!(
        legacy_entry.get("meta"),
        Some(&JsonValue::String(str_hex(
            meta.get_as_object().get_serializer().data()
        )))
    );
    assert!(!legacy_entry.contains_key("hash"));

    let v2 = get_json(
        &AppLedgerFill::new(
            &ledger,
            LedgerFillOptions::DUMP_TXRP | LedgerFillOptions::EXPAND | LedgerFillOptions::BINARY,
        )
        .with_transactions(&txs)
        .with_api_version(2),
    )
    .expect("v2 binary ledger json should render");
    let v2 = object(v2);
    let v2_entry = object(array(v2.get("transactions").expect("transactions"))[0].clone());
    assert_eq!(
        v2_entry.get("hash"),
        Some(&JsonValue::String(tx.get_transaction_id().to_string()))
    );
    assert_eq!(
        v2_entry.get("meta_blob"),
        Some(&JsonValue::String(str_hex(
            meta.get_as_object().get_serializer().data()
        )))
    );
}

#[test]
fn ledger_to_json_tx_v2_expanded_shapes_tx_json_meta_and_validation() {
    let ledger = base_ledger(44, true);
    let tx = payment_tx(9);
    let meta = payment_meta(
        tx.get_transaction_id(),
        44,
        Some(STAmount::new_native(321, false)),
    );
    let txs = [LedgerTxEntry {
        txn: &tx,
        meta: Some(&meta),
    }];
    let context = TestContext {
        api_version: 2,
        validated: true,
        close_time: Some(NetClockTimePoint::from(600_000_100)),
    };

    let rendered = get_json(
        &AppLedgerFill::new(
            &ledger,
            LedgerFillOptions::DUMP_TXRP | LedgerFillOptions::EXPAND,
        )
        .with_context(&context)
        .with_transactions(&txs),
    )
    .expect("v2 expanded ledger json should render");
    let rendered = object(rendered);
    let entry = object(array(rendered.get("transactions").expect("transactions"))[0].clone());

    assert_eq!(
        entry.get("hash"),
        Some(&JsonValue::String(tx.get_transaction_id().to_string()))
    );
    assert_eq!(
        entry.get("ledger_hash"),
        Some(&JsonValue::String(ledger.header().hash.to_string()))
    );
    assert_eq!(entry.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(entry.get("ledger_index"), Some(&JsonValue::Unsigned(44)));
    assert_eq!(
        entry.get("close_time_iso"),
        Some(&JsonValue::String("2019-01-05T10:41:40Z".to_string()))
    );

    let tx_json = object(
        entry
            .get("tx_json")
            .cloned()
            .expect("tx_json should be present"),
    );
    assert!(tx_json.contains_key("DeliverMax"));
    assert!(!tx_json.contains_key("Amount"));

    let meta_json = object(entry.get("meta").cloned().expect("meta should be present"));
    assert_eq!(
        meta_json.get("delivered_amount"),
        Some(&JsonValue::String("321".to_string()))
    );
}

#[test]
fn ledger_to_json_tx_legacy_expanded_keeps_amount_and_uses_metadata() {
    let ledger = base_ledger(44, true);
    let tx = payment_tx(10);
    let meta = payment_meta(
        tx.get_transaction_id(),
        44,
        Some(STAmount::new_native(7, false)),
    );
    let txs = [LedgerTxEntry {
        txn: &tx,
        meta: Some(&meta),
    }];

    let rendered = get_json(
        &AppLedgerFill::new(
            &ledger,
            LedgerFillOptions::DUMP_TXRP | LedgerFillOptions::EXPAND,
        )
        .with_transactions(&txs)
        .with_api_version(1),
    )
    .expect("legacy expanded ledger json should render");
    let rendered = object(rendered);
    let entry = object(array(rendered.get("transactions").expect("transactions"))[0].clone());

    assert!(entry.contains_key("hash"));
    assert!(entry.contains_key("Amount"));
    assert!(entry.contains_key("DeliverMax"));
    assert!(!entry.contains_key("validated"));
    assert!(!entry.contains_key("tx_json"));

    let meta_json = object(
        entry
            .get("metaData")
            .cloned()
            .expect("legacy metadata should be present"),
    );
    assert_eq!(
        meta_json.get("delivered_amount"),
        Some(&JsonValue::String("7".to_string()))
    );
}

#[test]
fn ledger_to_json_tx_inserts_mpt_issuance_id_for_successful_create() {
    let issuer = account("3333333333333333333333333333333333333333");
    let ledger = base_ledger(88, true);
    let tx = mpt_issuance_create_tx(12, issuer);
    let (meta, issuance_id) = mpt_meta(tx.get_transaction_id(), 88, 77, issuer);
    let txs = [LedgerTxEntry {
        txn: &tx,
        meta: Some(&meta),
    }];
    let context = TestContext {
        api_version: 2,
        validated: false,
        close_time: None,
    };

    let rendered = get_json(
        &AppLedgerFill::new(
            &ledger,
            LedgerFillOptions::DUMP_TXRP | LedgerFillOptions::EXPAND,
        )
        .with_context(&context)
        .with_transactions(&txs),
    )
    .expect("mpt create ledger json should render");
    let rendered = object(rendered);
    let entry = object(array(rendered.get("transactions").expect("transactions"))[0].clone());
    let meta_json = object(entry.get("meta").cloned().expect("meta should be present"));

    assert_eq!(
        meta_json.get("mpt_issuance_id"),
        Some(&JsonValue::String(issuance_id.to_string()))
    );
}

#[test]
fn ledger_to_json_tx_skips_delivered_amount_for_account_delete() {
    let ledger = base_ledger(90, true);
    let tx = account_delete_tx(13);
    let meta = payment_meta(
        tx.get_transaction_id(),
        90,
        Some(STAmount::new_native(123, false)),
    );
    let txs = [LedgerTxEntry {
        txn: &tx,
        meta: Some(&meta),
    }];

    let rendered = get_json(
        &AppLedgerFill::new(
            &ledger,
            LedgerFillOptions::DUMP_TXRP | LedgerFillOptions::EXPAND,
        )
        .with_api_version(2)
        .with_transactions(&txs),
    )
    .expect("account delete ledger json should render");
    let rendered = object(rendered);
    let entry = object(array(rendered.get("transactions").expect("transactions"))[0].clone());
    let meta_json = object(entry.get("meta").cloned().expect("meta should be present"));

    assert!(!meta_json.contains_key("delivered_amount"));
}
