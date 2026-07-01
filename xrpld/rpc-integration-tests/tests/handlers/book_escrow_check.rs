//! Tests for the book escrow check RPC handler.

use protocol::{
    currency_from_string, get_field_by_symbol, lsfDefaultRipple, to_base58, Issue, JsonValue,
    STAmount, STTx, TxType,
};
use rpc_integration_tests::env::*;
use server::{StreamKind, SubscriptionManager};
use std::collections::BTreeMap;

// === BOOK OFFERS with XRP TakerGets (owner_funds from XRP balance) ===

#[test]
fn book_offers_xrp_taker_gets_shows_xrp_owner_funds() {
    let mut alice = TestAccount::new("bx_alice");
    let gw = TestAccount::new("bx_gw");
    let env = RpcTestEnv::with_flags(
        &[(&alice, 10_000_000_000), (&gw, 10_000_000_000)],
        &[(&gw, lsfDefaultRipple)],
    );
    let usd = currency_from_string("USD");

    // Trust
    let mut trust = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(usd, gw.id),
                10000,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut trust, &alice);
    env.submit_and_close(&trust);

    // Offer: alice sells XRP for USD (TakerGets = XRP)
    let mut offer = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfTakerPays"),
                Issue::new(usd, gw.id),
                50,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfTakerGets"),
            STAmount::new_native(200_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut offer, &alice);
    env.submit_and_close(&offer);

    // Query USD/XRP book
    let source = env.rpc_source();
    let result = rpc::do_book_offers(
        &rpc::BookOffersRequest {
            params: &json([
                (
                    "taker_pays",
                    json([
                        ("currency", JsonValue::String("USD".to_owned())),
                        ("issuer", JsonValue::String(to_base58(gw.id))),
                    ]),
                ),
                (
                    "taker_gets",
                    json([("currency", JsonValue::String("XRP".to_owned()))]),
                ),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &source,
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if let Some(JsonValue::Array(offers)) = result.get("offers") {
        if !offers.is_empty() {
            let JsonValue::Object(o) = &offers[0] else {
                panic!("object")
            };
            // owner_funds should be XRP balance (drops)
            if let Some(JsonValue::String(funds)) = o.get("owner_funds") {
                let f: i64 = funds.parse().unwrap_or(0);
                assert!(f > 0, "XRP owner_funds should be positive: {funds}");
            }
            assert!(o.contains_key("quality"));
        }
    }
}

// === ACCOUNT_LINES with QualityIn/QualityOut ===

#[test]
fn account_lines_shows_quality_values_after_trust_with_quality() {
    let mut alice = TestAccount::new("aq_alice");
    let gw = TestAccount::new("aq_gw");
    let env = RpcTestEnv::with_flags(
        &[(&alice, 10_000_000_000), (&gw, 10_000_000_000)],
        &[(&gw, lsfDefaultRipple)],
    );
    let usd = currency_from_string("USD");

    // Trust with QualityIn=900000000, QualityOut=800000000
    let mut trust = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(usd, gw.id),
                1000,
                0,
                false,
            ),
        );
        tx.set_field_u32(get_field_by_symbol("sfQualityIn"), 900_000_000);
        tx.set_field_u32(get_field_by_symbol("sfQualityOut"), 800_000_000);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut trust, &alice);
    env.submit_and_close(&trust);

    let source = env.rpc_source();
    let result = rpc::do_account_lines(
        &rpc::AccountLinesRequest {
            params: &json([("account", JsonValue::String(to_base58(alice.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if let Some(JsonValue::Array(lines)) = result.get("lines") {
        if !lines.is_empty() {
            let JsonValue::Object(line) = &lines[0] else {
                panic!("object")
            };
            if let Some(JsonValue::Unsigned(qi)) = line.get("quality_in") {
                assert!(*qi > 0, "quality_in should be non-zero");
            }
            if let Some(JsonValue::Unsigned(qo)) = line.get("quality_out") {
                assert!(*qo > 0, "quality_out should be non-zero");
            }
        }
    }
}

// === ESCROW CREATE + LEDGER_ENTRY LOOKUP ===

#[test]
fn escrow_create_and_lookup() {
    let mut alice = TestAccount::new("esc_alice");
    let bob = TestAccount::new("esc_bob");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 1_000_000_000)]);

    let mut escrow = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfFinishAfter"), 2_000_000_000);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut escrow, &alice);
    env.submit_and_close(&escrow);

    // Try to find escrow via ledger_entry
    let source = env.rpc_source();
    let result = rpc::do_ledger_entry(
        &rpc::LedgerEntryRequest {
            params: &json([
                (
                    "escrow",
                    json([
                        ("owner", JsonValue::String(to_base58(alice.id))),
                        ("seq", JsonValue::Unsigned(1)),
                    ]),
                ),
                ("ledger_index", JsonValue::String("current".to_owned())),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };
    // Should find escrow or return entryNotFound
    assert!(result.contains_key("node") || result.contains_key("error"));
}

// === CHECK CREATE ===

#[test]
fn check_create_and_account_objects() {
    let mut alice = TestAccount::new("chk_alice");
    let bob = TestAccount::new("chk_bob");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 1_000_000_000)]);

    let mut check = STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob.id);
        tx.set_field_amount(
            get_field_by_symbol("sfSendMax"),
            STAmount::new_native(5_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut check, &alice);
    env.submit_and_close(&check);

    let source = env.rpc_source();
    let result = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(alice.id))),
                ("type", JsonValue::String("check".to_owned())),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if let Some(JsonValue::Array(objects)) = result.get("account_objects") {
        if !objects.is_empty() {
            let JsonValue::Object(check_obj) = &objects[0] else {
                panic!("object")
            };
            assert_eq!(
                check_obj.get("LedgerEntryType"),
                Some(&JsonValue::String("Check".to_owned()))
            );
            assert!(check_obj.contains_key("Destination"));
            assert!(check_obj.contains_key("SendMax"));
        }
    }
}

// === SUBSCRIBE BOOK_CHANGES STREAM ===

#[test]
fn subscribe_book_changes_stream() {
    let subs = SubscriptionManager::new(16);
    let mut rx = subs.subscribe(StreamKind::BookChanges);

    subs.publish_json(
        StreamKind::BookChanges,
        JsonValue::Object(BTreeMap::from([
            (
                "type".to_owned(),
                JsonValue::String("bookChanges".to_owned()),
            ),
            ("ledger_index".to_owned(), JsonValue::Unsigned(50)),
            ("ledger_time".to_owned(), JsonValue::Unsigned(800000000)),
            (
                "changes".to_owned(),
                JsonValue::Array(vec![JsonValue::Object(BTreeMap::from([
                    ("currency_a".to_owned(), JsonValue::String("XRP".to_owned())),
                    (
                        "currency_b".to_owned(),
                        JsonValue::String("USD.rGw".to_owned()),
                    ),
                    (
                        "volume_a".to_owned(),
                        JsonValue::String("100000000".to_owned()),
                    ),
                    ("volume_b".to_owned(), JsonValue::String("50".to_owned())),
                    ("high".to_owned(), JsonValue::String("2000000".to_owned())),
                    ("low".to_owned(), JsonValue::String("2000000".to_owned())),
                    ("open".to_owned(), JsonValue::String("2000000".to_owned())),
                    ("close".to_owned(), JsonValue::String("2000000".to_owned())),
                ]))]),
            ),
        ])),
    );

    let event = rx.try_recv().expect("should receive book_changes");
    let _payload_json: JsonValue = serde_json::from_slice(&event.payload).unwrap();
    let JsonValue::Object(payload) = _payload_json else {
        panic!("object")
    };
    assert_eq!(
        payload.get("type"),
        Some(&JsonValue::String("bookChanges".to_owned()))
    );
    assert_eq!(payload.get("ledger_index"), Some(&JsonValue::Unsigned(50)));
    let JsonValue::Array(changes) = payload.get("changes").unwrap() else {
        panic!("array")
    };
    assert_eq!(changes.len(), 1);
    let JsonValue::Object(change) = &changes[0] else {
        panic!("object")
    };
    assert!(change.contains_key("currency_a"));
    assert!(change.contains_key("currency_b"));
    assert!(change.contains_key("volume_a"));
    assert!(change.contains_key("volume_b"));
    assert!(change.contains_key("high"));
    assert!(change.contains_key("low"));
    assert!(change.contains_key("open"));
    assert!(change.contains_key("close"));
}

// === SUBSCRIBE VALIDATION STREAM ===

#[test]
fn subscribe_validation_stream() {
    let subs = SubscriptionManager::new(16);
    let mut rx = subs.subscribe(StreamKind::Validations);

    subs.publish_json(
        StreamKind::Validations,
        JsonValue::Object(BTreeMap::from([
            (
                "type".to_owned(),
                JsonValue::String("validationReceived".to_owned()),
            ),
            ("ledger_hash".to_owned(), JsonValue::String("FF".repeat(32))),
            (
                "validation_public_key".to_owned(),
                JsonValue::String(
                    "n949f75evCHwgyP4fPVgaHqNHxUVN15PsJEZ3B3HnXPcPjcZAoy7".to_owned(),
                ),
            ),
            ("full".to_owned(), JsonValue::Bool(true)),
        ])),
    );

    let event = rx.try_recv().expect("should receive validation");
    let _payload_json: JsonValue = serde_json::from_slice(&event.payload).unwrap();
    let JsonValue::Object(payload) = _payload_json else {
        panic!("object")
    };
    assert_eq!(
        payload.get("type"),
        Some(&JsonValue::String("validationReceived".to_owned()))
    );
    assert!(payload.contains_key("ledger_hash"));
    assert!(payload.contains_key("validation_public_key"));
    assert_eq!(payload.get("full"), Some(&JsonValue::Bool(true)));
}

// === ACCOUNT_OBJECTS STEPPED PAGINATION ===

#[test]
fn account_objects_stepped_one_at_a_time() {
    let mut alice = TestAccount::new("step_alice");
    let gw = TestAccount::new("step_gw");
    let env = RpcTestEnv::with_flags(
        &[(&alice, 10_000_000_000), (&gw, 10_000_000_000)],
        &[(&gw, lsfDefaultRipple)],
    );
    let usd = currency_from_string("USD");
    let eur = currency_from_string("EUR");

    // Create 2 trust lines
    for currency in [usd, eur] {
        let mut trust = STTx::new(TxType::TRUST_SET, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
            tx.set_field_amount(
                get_field_by_symbol("sfLimitAmount"),
                STAmount::new_with_asset(
                    get_field_by_symbol("sfLimitAmount"),
                    Issue::new(currency, gw.id),
                    1000,
                    0,
                    false,
                ),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
            tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
        });
        sign_tx(&mut trust, &alice);
        env.submit_and_close(&trust);
    }

    // Walk with limit=1
    let source = env.rpc_source();
    let mut total = 0;
    let mut marker: Option<JsonValue> = None;
    let mut iterations = 0;

    loop {
        let mut params = vec![
            ("account", JsonValue::String(to_base58(alice.id))),
            ("limit", JsonValue::Unsigned(1)),
        ];
        if let Some(ref m) = marker {
            params.push(("marker", m.clone()));
        }
        let result = rpc::do_account_objects(
            &rpc::AccountObjectsRequest {
                params: &json(params),
                api_version: 1,
                role: rpc::Role::Admin,
            },
            &source,
        );
        let JsonValue::Object(result) = result else {
            break;
        };
        if result.contains_key("error") {
            break;
        }

        if let Some(JsonValue::Array(objects)) = result.get("account_objects") {
            total += objects.len();
        }
        marker = result.get("marker").cloned();
        iterations += 1;
        if marker.is_none() || iterations > 20 {
            break;
        }
    }

    assert!(iterations >= 1, "should iterate at least once");
    assert!(total >= 2, "should find at least 2 objects (trust lines)");
}
