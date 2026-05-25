//! subscribe gateway tests part A.

use super::*;

#[test]
fn book_offers_multiple_quality_levels() {
    let mut alice = TestAccount::new("mq_alice");
    let mut bob = TestAccount::new("mq_bob");
    let mut gw = TestAccount::new("mq_gw");
    let env = RpcTestEnv::with_flags(
        &[
            (&alice, 10_000_000_000),
            (&bob, 10_000_000_000),
            (&gw, 10_000_000_000),
        ],
        &[(&gw, lsfDefaultRipple)],
    );
    let usd = currency_from_string("USD");

    // Setup trust + fund for both
    for acct in [&mut alice, &mut bob] {
        let mut t = STTx::new(TxType::TRUST_SET, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), acct.id);
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
            tx.set_field_u32(get_field_by_symbol("sfSequence"), acct.next_seq());
        });
        sign_tx(&mut t, acct);
        env.submit_and_close(&t);

        let mut p = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), gw.id);
            tx.set_account_id(get_field_by_symbol("sfDestination"), acct.id);
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_with_asset(
                    get_field_by_symbol("sfAmount"),
                    Issue::new(usd, gw.id),
                    1000,
                    0,
                    false,
                ),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
            tx.set_field_u32(get_field_by_symbol("sfSequence"), gw.next_seq());
        });
        sign_tx(&mut p, &gw);
        env.submit_and_close(&p);
    }

    // Alice: 100 USD for 200 XRP (quality = 2000000)
    let mut o1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(200_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfTakerGets"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfTakerGets"),
                Issue::new(usd, gw.id),
                100,
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
    sign_tx(&mut o1, &alice);
    env.submit_and_close(&o1);

    // Bob: 200 USD for 800 XRP (quality = 4000000)
    let mut o2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), bob.id);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(800_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfTakerGets"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfTakerGets"),
                Issue::new(usd, gw.id),
                200,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), bob.next_seq());
    });
    sign_tx(&mut o2, &bob);
    env.submit_and_close(&o2);

    let source = env.rpc_source();
    let result = rpc::do_book_offers(
        &rpc::BookOffersRequest {
            params: &json([
                (
                    "taker_pays",
                    json([("currency", JsonValue::String("XRP".to_owned()))]),
                ),
                (
                    "taker_gets",
                    json([
                        ("currency", JsonValue::String("USD".to_owned())),
                        ("issuer", JsonValue::String(to_base58(gw.id))),
                    ]),
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

    assert!(result.contains_key("offers") || result.contains_key("ledger_hash"));
    if let Some(JsonValue::Array(offers)) = result.get("offers") {
        // Each offer should have all required fields
        for offer in offers {
            let JsonValue::Object(o) = offer else {
                continue;
            };
            assert!(o.contains_key("Account"), "offer needs Account");
            assert!(o.contains_key("TakerPays"), "offer needs TakerPays");
            assert!(o.contains_key("TakerGets"), "offer needs TakerGets");
            assert!(o.contains_key("quality"), "offer needs quality");
            assert!(o.contains_key("owner_funds"), "offer needs owner_funds");

            assert!(o.contains_key("Sequence"), "offer needs Sequence");
            assert!(o.contains_key("BookDirectory"), "offer needs BookDirectory");
            assert!(o.contains_key("index"), "offer needs index");
        }
    }
}

// === SUBSCRIBE - all stream types ===

#[test]
fn subscribe_peer_status_stream() {
    let subs = SubscriptionManager::new(16);
    let mut rx = subs.subscribe(StreamKind::PeerStatus);
    subs.publish_json(
        StreamKind::PeerStatus,
        JsonValue::Object(BTreeMap::from([
            (
                "type".to_owned(),
                JsonValue::String("peerStatusChange".to_owned()),
            ),
            ("action".to_owned(), JsonValue::String("connect".to_owned())),
            (
                "address".to_owned(),
                JsonValue::String("192.168.1.1:51235".to_owned()),
            ),
        ])),
    );
    let event = rx.try_recv().expect("should receive");
    let JsonValue::Object(p) = event.payload else {
        panic!("object")
    };
    assert_eq!(
        p.get("type"),
        Some(&JsonValue::String("peerStatusChange".to_owned()))
    );
    assert_eq!(
        p.get("action"),
        Some(&JsonValue::String("connect".to_owned()))
    );
    assert!(p.contains_key("address"));
}

#[test]
fn subscribe_multiple_receivers_same_stream() {
    let subs = SubscriptionManager::new(16);
    let mut rx1 = subs.subscribe(StreamKind::Ledger);
    let mut rx2 = subs.subscribe(StreamKind::Ledger);

    subs.publish_json(
        StreamKind::Ledger,
        JsonValue::Object(BTreeMap::from([
            (
                "type".to_owned(),
                JsonValue::String("ledgerClosed".to_owned()),
            ),
            ("ledger_index".to_owned(), JsonValue::Unsigned(99)),
        ])),
    );

    let e1 = rx1.try_recv().expect("rx1 should receive");
    let e2 = rx2.try_recv().expect("rx2 should receive");
    let JsonValue::Object(p1) = e1.payload else {
        panic!("object")
    };
    let JsonValue::Object(p2) = e2.payload else {
        panic!("object")
    };
    assert_eq!(p1.get("ledger_index"), Some(&JsonValue::Unsigned(99)));
    assert_eq!(p2.get("ledger_index"), Some(&JsonValue::Unsigned(99)));
}

#[test]
fn subscribe_high_volume_events() {
    let subs = SubscriptionManager::new(64);
    let mut rx = subs.subscribe(StreamKind::Transactions);

    for i in 0..50u64 {
        subs.publish_json(
            StreamKind::Transactions,
            JsonValue::Object(BTreeMap::from([
                (
                    "type".to_owned(),
                    JsonValue::String("transaction".to_owned()),
                ),
                ("seq".to_owned(), JsonValue::Unsigned(i)),
            ])),
        );
    }

    let mut count = 0;
    while rx.try_recv().is_ok() {
        count += 1;
    }
    assert_eq!(count, 50, "should receive all 50 events");
}

// === ACCOUNT_LINES - multiple gateways ===

#[test]
fn account_lines_from_multiple_gateways() {
    let mut alice = TestAccount::new("mg_alice");
    let gw1 = TestAccount::new("mg_gw1");
    let gw2 = TestAccount::new("mg_gw2");
    let env = RpcTestEnv::with_flags(
        &[
            (&alice, 10_000_000_000),
            (&gw1, 10_000_000_000),
            (&gw2, 10_000_000_000),
        ],
        &[(&gw1, lsfDefaultRipple), (&gw2, lsfDefaultRipple)],
    );

    // Trust gw1 for USD
    let mut t1 = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(currency_from_string("USD"), gw1.id),
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
    sign_tx(&mut t1, &alice);
    env.submit_and_close(&t1);

    // Trust gw2 for EUR
    let mut t2 = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(currency_from_string("EUR"), gw2.id),
                2000,
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
    sign_tx(&mut t2, &alice);
    env.submit_and_close(&t2);

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
        assert!(lines.len() >= 2, "should have at least 2 trust lines");
        for line in lines {
            let JsonValue::Object(l) = line else { continue };
            assert!(l.contains_key("account"));
            assert!(l.contains_key("balance"));
            assert!(l.contains_key("currency"));
            assert!(l.contains_key("limit"));
            assert!(l.contains_key("limit_peer"));
            assert!(l.contains_key("quality_in"));
            assert!(l.contains_key("quality_out"));
        }
    }
}

// === GATEWAY BALANCES with hotwallet ===
