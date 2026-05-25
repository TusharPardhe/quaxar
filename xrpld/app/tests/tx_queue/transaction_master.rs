use app::{SharedTransaction, TransStatus, Transaction, TransactionFetchResult, TransactionMaster};
use basics::base_uint::Uint256;
use basics::range_set::ClosedInterval;
use basics::tagged_cache::ManualClock;
use protocol::{
    STAmount, STArray, STObject, STTx, TxMeta, TxSearched, TxType, get_field_by_symbol,
};
use shamap::item::SHAMapItem;
use shamap::tree_node::SHAMapNodeType;
use std::sync::{Arc, Mutex};

fn account(fill: u8) -> protocol::AccountID {
    protocol::AccountID::from_array([fill; 20])
}

fn payment_tx(sequence: u32, fill: u8) -> STTx {
    STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(fill));
        tx.set_account_id(
            get_field_by_symbol("sfDestination"),
            account(fill.wrapping_add(1)),
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

fn shared_transaction(tx: STTx) -> SharedTransaction {
    Arc::new(Mutex::new(Transaction::new(Arc::new(tx))))
}

fn sample_meta(tx_id: Uint256) -> TxMeta {
    let affected_nodes = STArray::new(get_field_by_symbol("sfAffectedNodes"));
    let mut meta = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    meta.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    meta.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 1);
    meta.set_field_array(get_field_by_symbol("sfAffectedNodes"), affected_nodes);
    TxMeta::from_stobject(tx_id, 1, meta)
}

#[test]
fn transaction_master_in_ledger_updates_cached_transaction() {
    let master = TransactionMaster::new_with_clock(ManualClock::new(0));
    let tx = payment_tx(5, 0x11);
    let mut shared = shared_transaction(tx.clone());

    master.canonicalize(&mut shared);

    assert!(master.in_ledger(tx.get_transaction_id(), 99, Some(7), Some(8)));
    let cached = master
        .fetch_from_cache(&tx.get_transaction_id())
        .expect("cached transaction should exist");
    let cached = cached
        .lock()
        .expect("transaction mutex must not be poisoned");

    assert_eq!(cached.get_status(), TransStatus::COMMITTED);
    assert_eq!(cached.get_ledger(), 99);
    assert!(cached.is_validated());
    let json = cached.get_json(protocol::JsonOptions::NONE, false);
    let protocol::JsonValue::Object(json) = json else {
        panic!("transaction JSON should remain an object");
    };
    assert_eq!(
        json.get("ctid"),
        Some(&protocol::JsonValue::String("C000006300070008".to_owned()))
    );
}

#[test]
fn transaction_master_in_ledger_returns_false_on_cache_miss() {
    let master = TransactionMaster::new_with_clock(ManualClock::new(0));

    assert!(!master.in_ledger(Uint256::from_u64(1), 99, Some(7), Some(8)));
}

#[test]
fn transaction_master_canonicalize_reuses_cached_owner() {
    let master = TransactionMaster::new_with_clock(ManualClock::new(0));
    let tx = payment_tx(6, 0x21);
    let mut first = shared_transaction(tx.clone());
    master.canonicalize(&mut first);

    let mut second = shared_transaction(tx);
    master.canonicalize(&mut second);

    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(master.get_cache().size(), 1);
}

#[test]
fn transaction_master_fetch_skips_loader_for_unvalidated_cache() {
    let master = TransactionMaster::new_with_clock(ManualClock::new(0));
    let tx = payment_tx(7, 0x31);
    let mut cached = shared_transaction(tx.clone());
    master.canonicalize(&mut cached);

    let mut called = false;
    let result = master
        .fetch(tx.get_transaction_id(), || {
            called = true;
            Ok::<_, ()>(TransactionFetchResult::NotFound(TxSearched::Unknown))
        })
        .expect("cache fetch should succeed");

    let TransactionFetchResult::Found(result) = result else {
        panic!("cache hit should return a transaction");
    };

    assert!(!called);
    assert!(Arc::ptr_eq(&cached, &result.0));
    assert!(result.1.is_none());
}

#[test]
fn transaction_master_fetch_canonicalizes_loaded_transaction() {
    let master = TransactionMaster::new_with_clock(ManualClock::new(0));
    let tx = payment_tx(8, 0x41);
    let mut cached = shared_transaction(tx.clone());
    cached
        .lock()
        .expect("transaction mutex must not be poisoned")
        .set_ledger(1);
    master.canonicalize(&mut cached);

    let loaded = shared_transaction(tx.clone());
    let meta = sample_meta(tx.get_transaction_id());
    let result = master
        .fetch(tx.get_transaction_id(), || {
            Ok::<_, ()>(TransactionFetchResult::Found((loaded, Some(meta))))
        })
        .expect("loaded fetch should succeed");

    let TransactionFetchResult::Found(result) = result else {
        panic!("loaded transaction should be returned");
    };

    assert!(Arc::ptr_eq(&cached, &result.0));
    assert!(result.1.is_some());
}

#[test]
fn transaction_master_fetch_calls_loader_for_validated_cache() {
    let master = TransactionMaster::new_with_clock(ManualClock::new(0));
    let tx = payment_tx(8, 0x43);
    let mut cached = shared_transaction(tx.clone());
    cached
        .lock()
        .expect("transaction mutex must not be poisoned")
        .set_ledger(1);
    master.canonicalize(&mut cached);

    let mut called = false;
    let result = master
        .fetch(tx.get_transaction_id(), || {
            called = true;
            Ok::<_, ()>(TransactionFetchResult::NotFound(TxSearched::Unknown))
        })
        .expect("validated cached fetch should succeed");

    assert!(called);
    assert!(matches!(
        result,
        TransactionFetchResult::NotFound(TxSearched::Unknown)
    ));
}

#[test]
fn transaction_master_fetch_in_range_passes_search_window() {
    let master = TransactionMaster::new_with_clock(ManualClock::new(0));
    let tx = payment_tx(8, 0x42);
    let range = ClosedInterval::new(10, 20);
    let mut saw_range = None;

    let result = master
        .fetch_in_range(tx.get_transaction_id(), range, |passed_range| {
            saw_range = Some((passed_range.first(), passed_range.last()));
            Ok::<_, ()>(TransactionFetchResult::NotFound(TxSearched::Some))
        })
        .expect("ranged fetch should succeed");

    assert_eq!(saw_range, Some((10, 20)));
    assert!(matches!(
        result,
        TransactionFetchResult::NotFound(TxSearched::Some)
    ));
}

#[test]
fn transaction_master_fetch_from_transaction_nodes_decode_rules() {
    let master = TransactionMaster::new_with_clock(ManualClock::new(0));
    let tx = payment_tx(9, 0x51);
    let raw = tx.get_serializer().data().to_vec();
    let item = SHAMapItem::new(tx.get_transaction_id(), raw.clone());

    let from_nm = master
        .fetch_from_shamap_item(&item, SHAMapNodeType::TransactionNm, 0)
        .expect("transaction node should parse");
    let from_nm = from_nm.expect("transaction node should return a transaction");
    assert_eq!(from_nm.get_transaction_id(), tx.get_transaction_id());

    let mut payload = Vec::new();
    let mut serializer = protocol::Serializer::new(0);
    serializer.add_vl(&raw);
    payload.extend_from_slice(serializer.data());
    payload.extend_from_slice(&[0xAA, 0xBB]);
    let md_item = SHAMapItem::new(tx.get_transaction_id(), payload);
    let from_md = master
        .fetch_from_shamap_item(&md_item, SHAMapNodeType::TransactionMd, 0)
        .expect("transaction-with-meta node should parse");
    let from_md = from_md.expect("transaction-with-meta node should return a transaction");
    assert_eq!(from_md.get_transaction_id(), tx.get_transaction_id());
}

#[test]
fn transaction_master_fetch_from_cached_item_promotes_commit_ledger() {
    let master = TransactionMaster::new_with_clock(ManualClock::new(0));
    let tx = payment_tx(10, 0x61);
    let mut cached = shared_transaction(tx.clone());
    master.canonicalize(&mut cached);

    let bogus_item = SHAMapItem::new(tx.get_transaction_id(), vec![0xFF]);
    let fetched = master
        .fetch_from_shamap_item(&bogus_item, SHAMapNodeType::TransactionNm, 500)
        .expect("cache hit should bypass payload parsing");
    let fetched = fetched.expect("cache hit should return the cached transaction");
    assert_eq!(fetched.get_transaction_id(), tx.get_transaction_id());
    assert_eq!(
        cached
            .lock()
            .expect("transaction mutex must not be poisoned")
            .get_status(),
        TransStatus::COMMITTED
    );
    assert_eq!(
        cached
            .lock()
            .expect("transaction mutex must not be poisoned")
            .get_ledger(),
        500
    );
}

#[test]
fn transaction_master_fetch_from_cached_item_skips_zero_commit_ledger_promotion() {
    let master = TransactionMaster::new_with_clock(ManualClock::new(0));
    let tx = payment_tx(10, 0x62);
    let mut cached = shared_transaction(tx.clone());
    master.canonicalize(&mut cached);

    let bogus_item = SHAMapItem::new(tx.get_transaction_id(), vec![0xFF]);
    let fetched = master
        .fetch_from_shamap_item(&bogus_item, SHAMapNodeType::TransactionNm, 0)
        .expect("cache hit should bypass payload parsing");
    let fetched = fetched.expect("cache hit should return the cached transaction");
    assert_eq!(fetched.get_transaction_id(), tx.get_transaction_id());
    let cached = cached
        .lock()
        .expect("transaction mutex must not be poisoned");
    assert_eq!(cached.get_status(), TransStatus::NEW);
    assert_eq!(cached.get_ledger(), 0);
}

#[test]
fn transaction_master_returns_none_for_unsupported_shamap_node_types() {
    let master = TransactionMaster::new_with_clock(ManualClock::new(0));
    let item = SHAMapItem::new(Uint256::from_u64(1), vec![0, 1, 2, 3]);

    let fetched = master
        .fetch_from_shamap_item(&item, SHAMapNodeType::AccountState, 0)
        .expect("unsupported non-transaction nodes should not error");

    assert!(fetched.is_none());
}
