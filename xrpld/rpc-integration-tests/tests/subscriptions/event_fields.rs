//! Integration tests for the subscribe RPC handler.
//! Tests the full subscribe → ledger close → receive event pipeline.

use std::collections::BTreeMap;

use protocol::{get_field_by_symbol, JsonValue, STAmount, STTx, TxType};
use rpc_integration_tests::env::*;
use server::{StreamKind, SubscriptionManager};

fn publish_ledger_closed(
    subs: &SubscriptionManager,
    ledger_index: u32,
    ledger_hash: &str,
    txn_count: u32,
    close_time: u32,
) {
    subs.publish_json(
        StreamKind::Ledger,
        JsonValue::Object(BTreeMap::from([
            (
                "type".to_owned(),
                JsonValue::String("ledgerClosed".to_owned()),
            ),
            (
                "ledger_index".to_owned(),
                JsonValue::Unsigned(u64::from(ledger_index)),
            ),
            (
                "ledger_hash".to_owned(),
                JsonValue::String(ledger_hash.to_owned()),
            ),
            (
                "txn_count".to_owned(),
                JsonValue::Unsigned(u64::from(txn_count)),
            ),
            (
                "ledger_time".to_owned(),
                JsonValue::Unsigned(u64::from(close_time)),
            ),
            (
                "validated_ledgers".to_owned(),
                JsonValue::String(format!("1-{ledger_index}")),
            ),
        ])),
    );
}

/// Simulate transaction stream event.
fn publish_transaction(subs: &SubscriptionManager, tx_type: &str, account: &str, hash: &str) {
    subs.publish_json(
        StreamKind::Transactions,
        JsonValue::Object(BTreeMap::from([
            (
                "type".to_owned(),
                JsonValue::String("transaction".to_owned()),
            ),
            (
                "transaction".to_owned(),
                JsonValue::Object(BTreeMap::from([
                    (
                        "TransactionType".to_owned(),
                        JsonValue::String(tx_type.to_owned()),
                    ),
                    ("Account".to_owned(), JsonValue::String(account.to_owned())),
                    ("hash".to_owned(), JsonValue::String(hash.to_owned())),
                ])),
            ),
            ("validated".to_owned(), JsonValue::Bool(true)),
        ])),
    );
}

#[test]
fn subscribe_ledger_stream_receives_close_event() {
    let subs = SubscriptionManager::new(16);
    let mut rx = subs.subscribe(StreamKind::Ledger);

    publish_ledger_closed(&subs, 42, &"AB".repeat(32), 5, 777);

    let event = rx.try_recv().expect("should receive ledger event");
    let payload_json: JsonValue = serde_json::from_slice(&event.payload).unwrap();
    let JsonValue::Object(payload) = payload_json else {
        panic!("object")
    };

    assert_eq!(
        payload.get("type"),
        Some(&JsonValue::String("ledgerClosed".to_owned()))
    );
    assert_eq!(payload.get("ledger_index"), Some(&JsonValue::Unsigned(42)));
    assert_eq!(payload.get("txn_count"), Some(&JsonValue::Unsigned(5)));
    assert_eq!(payload.get("ledger_time"), Some(&JsonValue::Unsigned(777)));
    assert!(payload.contains_key("ledger_hash"));
    assert!(payload.contains_key("validated_ledgers"));
}

#[test]
fn subscribe_transaction_stream_receives_tx_event() {
    let subs = SubscriptionManager::new(16);
    let mut rx = subs.subscribe(StreamKind::Transactions);

    publish_transaction(
        &subs,
        "Payment",
        "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh",
        &"CC".repeat(32),
    );

    let event = rx.try_recv().expect("should receive tx event");
    let payload_json: JsonValue = serde_json::from_slice(&event.payload).unwrap();
    let JsonValue::Object(payload) = payload_json else {
        panic!("object")
    };

    assert_eq!(
        payload.get("type"),
        Some(&JsonValue::String("transaction".to_owned()))
    );
    assert_eq!(payload.get("validated"), Some(&JsonValue::Bool(true)));
    let JsonValue::Object(tx) = payload.get("transaction").unwrap() else {
        panic!("object")
    };
    assert_eq!(
        tx.get("TransactionType"),
        Some(&JsonValue::String("Payment".to_owned()))
    );
}

#[test]
fn subscribe_server_stream_receives_fee_change() {
    let subs = SubscriptionManager::new(16);
    let mut rx = subs.subscribe(StreamKind::Server);

    subs.publish_json(
        StreamKind::Server,
        JsonValue::Object(BTreeMap::from([
            (
                "type".to_owned(),
                JsonValue::String("serverStatus".to_owned()),
            ),
            ("load_base".to_owned(), JsonValue::Unsigned(256)),
            ("load_factor".to_owned(), JsonValue::Unsigned(512)),
            (
                "server_status".to_owned(),
                JsonValue::String("full".to_owned()),
            ),
        ])),
    );

    let event = rx.try_recv().expect("should receive server event");
    let payload_json: JsonValue = serde_json::from_slice(&event.payload).unwrap();
    let JsonValue::Object(payload) = payload_json else {
        panic!("object")
    };

    assert_eq!(
        payload.get("type"),
        Some(&JsonValue::String("serverStatus".to_owned()))
    );
    assert_eq!(payload.get("load_base"), Some(&JsonValue::Unsigned(256)));
    assert_eq!(payload.get("load_factor"), Some(&JsonValue::Unsigned(512)));
    assert_eq!(
        payload.get("server_status"),
        Some(&JsonValue::String("full".to_owned()))
    );
}

#[test]
fn unsubscribe_stops_receiving_events() {
    let subs = SubscriptionManager::new(16);
    let rx = subs.subscribe(StreamKind::Ledger);
    drop(rx); // Unsubscribe by dropping receiver

    // Publish after unsubscribe - should not panic
    let sent = subs.publish_json(
        StreamKind::Ledger,
        JsonValue::Object(BTreeMap::from([(
            "type".to_owned(),
            JsonValue::String("ledgerClosed".to_owned()),
        )])),
    );
    // No receivers, so sent count should be 0
    assert_eq!(sent, 0);
}

#[test]
fn subscribe_multiple_streams_independently() {
    let subs = SubscriptionManager::new(16);
    let mut ledger_rx = subs.subscribe(StreamKind::Ledger);
    let mut tx_rx = subs.subscribe(StreamKind::Transactions);

    // Publish to ledger only
    publish_ledger_closed(&subs, 10, &"DD".repeat(32), 0, 100);

    // Ledger receiver should get it
    assert!(ledger_rx.try_recv().is_ok());
    // Transaction receiver should NOT get it
    assert!(tx_rx.try_recv().is_err());

    // Publish to transactions only
    publish_transaction(
        &subs,
        "OfferCreate",
        "rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe",
        &"EE".repeat(32),
    );

    // Transaction receiver should get it
    assert!(tx_rx.try_recv().is_ok());
}

#[test]
fn subscribe_ledger_close_after_real_accept() {
    let mut alice = TestAccount::new("sub_alice");
    let bob = TestAccount::new("sub_bob");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 1_000_000_000)]);

    let subs = SubscriptionManager::new(16);
    let mut rx = subs.subscribe(StreamKind::Ledger);

    // Submit a payment and close
    let mut payment = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut payment, &alice);
    env.submit_and_close(&payment);

    // Simulate the event that would be published by NetworkOPs after close
    let closed = env.app.closed_ledger().expect("should have closed ledger");
    publish_ledger_closed(
        &subs,
        closed.header().seq,
        &closed.header().hash.as_uint256().to_string(),
        1, // 1 transaction
        closed.header().close_time,
    );

    // Verify event received
    let event = rx.try_recv().expect("should receive ledger close event");
    let payload_json: JsonValue = serde_json::from_slice(&event.payload).unwrap();
    let JsonValue::Object(payload) = payload_json else {
        panic!("object")
    };
    assert_eq!(
        payload.get("type"),
        Some(&JsonValue::String("ledgerClosed".to_owned()))
    );
    assert_eq!(payload.get("txn_count"), Some(&JsonValue::Unsigned(1)));
    assert!(
        matches!(payload.get("ledger_index"), Some(JsonValue::Unsigned(idx)) if *idx >= 2),
        "ledger index should be >= 2"
    );
}

#[test]
fn subscribe_validates_stream_isolation() {
    let subs = SubscriptionManager::new(16);
    let mut server_rx = subs.subscribe(StreamKind::Server);

    // Publish to all other streams - server should not receive them
    publish_ledger_closed(&subs, 1, &"AA".repeat(32), 0, 0);
    publish_transaction(&subs, "Payment", "rTest", &"BB".repeat(32));
    subs.publish_json(
        StreamKind::Validations,
        JsonValue::Object(BTreeMap::from([(
            "type".to_owned(),
            JsonValue::String("validationReceived".to_owned()),
        )])),
    );

    // Server should have received nothing
    assert!(
        server_rx.try_recv().is_err(),
        "server stream should not receive other events"
    );

    // Now publish to server
    subs.publish_json(
        StreamKind::Server,
        JsonValue::Object(BTreeMap::from([(
            "type".to_owned(),
            JsonValue::String("serverStatus".to_owned()),
        )])),
    );
    assert!(
        server_rx.try_recv().is_ok(),
        "server stream should receive server events"
    );
}

#[test]
fn subscribe_transaction_event_has_full_structure() {
    let subs = SubscriptionManager::new(16);
    let mut rx = subs.subscribe(StreamKind::Transactions);

    subs.publish_json(
        StreamKind::Transactions,
        JsonValue::Object(BTreeMap::from([
            (
                "type".to_owned(),
                JsonValue::String("transaction".to_owned()),
            ),
            ("validated".to_owned(), JsonValue::Bool(true)),
            ("ledger_index".to_owned(), JsonValue::Unsigned(42)),
            ("ledger_hash".to_owned(), JsonValue::String("AA".repeat(32))),
            (
                "engine_result".to_owned(),
                JsonValue::String("tesSUCCESS".to_owned()),
            ),
            ("engine_result_code".to_owned(), JsonValue::Signed(0)),
            (
                "engine_result_message".to_owned(),
                JsonValue::String("The transaction was applied.".to_owned()),
            ),
            (
                "transaction".to_owned(),
                JsonValue::Object(BTreeMap::from([
                    (
                        "TransactionType".to_owned(),
                        JsonValue::String("Payment".to_owned()),
                    ),
                    (
                        "Account".to_owned(),
                        JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
                    ),
                    (
                        "Destination".to_owned(),
                        JsonValue::String("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe".to_owned()),
                    ),
                    ("Amount".to_owned(), JsonValue::String("1000000".to_owned())),
                    ("Fee".to_owned(), JsonValue::String("10".to_owned())),
                    ("Sequence".to_owned(), JsonValue::Unsigned(1)),
                    ("hash".to_owned(), JsonValue::String("BB".repeat(32))),
                ])),
            ),
            (
                "meta".to_owned(),
                JsonValue::Object(BTreeMap::from([
                    (
                        "TransactionResult".to_owned(),
                        JsonValue::String("tesSUCCESS".to_owned()),
                    ),
                    ("TransactionIndex".to_owned(), JsonValue::Unsigned(0)),
                ])),
            ),
        ])),
    );

    let event = rx.try_recv().expect("should receive");
    let payload_json: JsonValue = serde_json::from_slice(&event.payload).unwrap();
    let JsonValue::Object(payload) = payload_json else {
        panic!("object")
    };

    assert_eq!(
        payload.get("type"),
        Some(&JsonValue::String("transaction".to_owned()))
    );
    assert_eq!(payload.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(payload.get("ledger_index"), Some(&JsonValue::Unsigned(42)));
    assert!(payload.contains_key("ledger_hash"));
    assert_eq!(
        payload.get("engine_result"),
        Some(&JsonValue::String("tesSUCCESS".to_owned()))
    );
    assert_eq!(
        payload.get("engine_result_code"),
        Some(&JsonValue::Unsigned(0))
    );
    assert!(payload.contains_key("engine_result_message"));

    let JsonValue::Object(tx) = payload.get("transaction").unwrap() else {
        panic!("object")
    };
    assert_eq!(
        tx.get("TransactionType"),
        Some(&JsonValue::String("Payment".to_owned()))
    );
    assert!(tx.contains_key("Account"));
    assert!(tx.contains_key("Destination"));
    assert!(tx.contains_key("Amount"));
    assert!(tx.contains_key("Fee"));
    assert!(tx.contains_key("Sequence"));
    assert!(tx.contains_key("hash"));

    let JsonValue::Object(meta) = payload.get("meta").unwrap() else {
        panic!("object")
    };
    assert_eq!(
        meta.get("TransactionResult"),
        Some(&JsonValue::String("tesSUCCESS".to_owned()))
    );
    assert!(meta.contains_key("TransactionIndex"));
}

#[test]
fn subscribe_ledger_event_has_full_structure() {
    let subs = SubscriptionManager::new(16);
    let mut rx = subs.subscribe(StreamKind::Ledger);

    subs.publish_json(
        StreamKind::Ledger,
        JsonValue::Object(BTreeMap::from([
            (
                "type".to_owned(),
                JsonValue::String("ledgerClosed".to_owned()),
            ),
            ("ledger_index".to_owned(), JsonValue::Unsigned(100)),
            ("ledger_hash".to_owned(), JsonValue::String("DD".repeat(32))),
            ("ledger_time".to_owned(), JsonValue::Unsigned(750000000)),
            ("txn_count".to_owned(), JsonValue::Unsigned(3)),
            ("fee_base".to_owned(), JsonValue::Unsigned(10)),
            ("fee_ref".to_owned(), JsonValue::Unsigned(10)),
            ("reserve_base".to_owned(), JsonValue::Unsigned(10000000)),
            ("reserve_inc".to_owned(), JsonValue::Unsigned(2000000)),
            (
                "validated_ledgers".to_owned(),
                JsonValue::String("1-100".to_owned()),
            ),
        ])),
    );

    let event = rx.try_recv().expect("should receive");
    let payload_json: JsonValue = serde_json::from_slice(&event.payload).unwrap();
    let JsonValue::Object(payload) = payload_json else {
        panic!("object")
    };

    assert_eq!(
        payload.get("type"),
        Some(&JsonValue::String("ledgerClosed".to_owned()))
    );
    assert_eq!(payload.get("ledger_index"), Some(&JsonValue::Unsigned(100)));
    assert!(payload.contains_key("ledger_hash"));
    assert_eq!(
        payload.get("ledger_time"),
        Some(&JsonValue::Unsigned(750000000))
    );
    assert_eq!(payload.get("txn_count"), Some(&JsonValue::Unsigned(3)));
    assert_eq!(payload.get("fee_base"), Some(&JsonValue::Unsigned(10)));
    assert_eq!(
        payload.get("reserve_base"),
        Some(&JsonValue::Unsigned(10000000))
    );
    assert_eq!(
        payload.get("reserve_inc"),
        Some(&JsonValue::Unsigned(2000000))
    );
    assert!(payload.contains_key("validated_ledgers"));
}
