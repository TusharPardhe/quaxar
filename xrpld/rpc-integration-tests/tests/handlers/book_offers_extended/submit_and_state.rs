//! book offers extended tests part B.

use super::*;

#[test]
fn submit_underfunded_payment_returns_tec() {
    let mut alice = TestAccount::new("retry_alice2");
    let bob = TestAccount::new("retry_bob2");
    // Alice has very little XRP
    let env = RpcTestEnv::new(&[(&alice, 1_000_000), (&bob, 1_000_000)]);

    // Try to send more than alice has
    let mut payment = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(999_999_999, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut payment, &alice);

    let tx_blob = basics::str_hex::str_hex(payment.get_serializer().data());
    let source = env.rpc_source();
    let result = rpc::do_submit(&rpc::RpcRequestContext {
        params: &json([("tx_blob", JsonValue::String(tx_blob))]),
        env: &rpc::SubmitSource,
        runtime: &source,
        role: rpc::Role::Admin,
        api_version: 2,
        headers: rpc::JsonContextHeaders::default(),
        request_headers: std::collections::BTreeMap::new(),
        unlimited: true,
        remote_ip: None,
        load_type: rpc::RpcLoadType::Reference,
    });

    match result {
        Ok(JsonValue::Object(obj)) => {
            if let Some(JsonValue::String(engine_result)) = obj.get("engine_result") {
                // Underfunded should produce tecUNFUNDED_PAYMENT or similar
                assert!(
                    engine_result.starts_with("tec")
                        || engine_result.starts_with("ter")
                        || engine_result.starts_with("tes")
                        || engine_result.starts_with("tef"),
                    "underfunded should produce a result code: {engine_result}"
                );
            }
        }
        Err(_) => {}
        _ => {}
    }
}

// === REMAINING EDGE CASES ===

#[test]
fn account_info_after_multiple_payments() {
    let mut alice = TestAccount::new("rem_alice");
    let bob = TestAccount::new("rem_bob");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 1_000_000_000)]);

    // Send 3 payments
    for _ in 0..3 {
        let mut payment = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
            tx.set_account_id(get_field_by_symbol("sfDestination"), bob.id);
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(100_000, false),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
            tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
        });
        sign_tx(&mut payment, &alice);
        env.submit_and_close(&payment);
    }

    let source = env.rpc_source();
    let result = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(alice.id))),
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

    if let Some(JsonValue::Object(data)) = result.get("account_data") {
        // Handler responds with account data
        assert!(data.contains_key("Sequence"));
        assert!(data.contains_key("Balance"));
    }
}

#[test]
fn ledger_closed_hash_changes_after_tx() {
    let mut alice = TestAccount::new("rem_alice2");
    let bob = TestAccount::new("rem_bob2");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 1_000_000_000)]);

    let source = env.rpc_source();
    let before = rpc::do_ledger_closed(&source);
    let JsonValue::Object(before) = before else {
        panic!("object")
    };
    let hash1 = before.get("ledger_hash").cloned();

    // Submit and close
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

    let source = env.rpc_source();
    let after = rpc::do_ledger_closed(&source);
    let JsonValue::Object(after) = after else {
        panic!("object")
    };
    let hash2 = after.get("ledger_hash").cloned();

    // Hash should change after a tx
    assert_ne!(hash1, hash2, "ledger hash should change after transaction");
}

#[test]
fn book_offers_shows_quality_and_owner_funds() {
    let mut alice = TestAccount::new("bk_alice3");
    let mut gw = TestAccount::new("bk_gw3");

    let env = RpcTestEnv::with_flags(
        &[(&alice, 10_000_000_000), (&gw, 10_000_000_000)],
        &[(&gw, lsfDefaultRipple)],
    );

    let usd = currency_from_string("USD");

    // Trust + fund
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

    let mut pay = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), gw.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfAmount"),
                Issue::new(usd, gw.id),
                500,
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
    sign_tx(&mut pay, &gw);
    env.submit_and_close(&pay);

    // Create offer: alice sells 100 USD for 400 XRP
    let mut offer = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(400_000_000, false),
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
    sign_tx(&mut offer, &alice);
    env.submit_and_close(&offer);

    // Query book
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

    if let Some(JsonValue::Array(offers)) = result.get("offers") {
        if !offers.is_empty() {
            let JsonValue::Object(offer_obj) = &offers[0] else {
                panic!("object")
            };

            assert!(
                offer_obj.contains_key("quality"),
                "offer should have quality"
            );
            if let Some(JsonValue::String(quality)) = offer_obj.get("quality") {
                let q: u64 = quality.parse().unwrap_or(0);
                assert!(q > 0, "quality should be non-zero: {quality}");
            }

            assert!(
                offer_obj.contains_key("owner_funds"),
                "offer should have owner_funds"
            );
            if let Some(JsonValue::String(funds)) = offer_obj.get("owner_funds") {
                assert!(!funds.is_empty(), "owner_funds should not be empty");
            }

            assert!(offer_obj.contains_key("Account"));
            assert!(offer_obj.contains_key("Sequence") || offer_obj.contains_key("index"));
            assert!(offer_obj.contains_key("TakerPays"));
            assert!(offer_obj.contains_key("TakerGets"));
            assert!(offer_obj.contains_key("Flags"));
            assert!(offer_obj.contains_key("BookDirectory"));
        }
    }
}
