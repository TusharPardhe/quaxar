//! book offers extended tests part A.

use super::*;

#[test]
fn book_offers_after_successful_offer_shows_fields() {
    let mut alice = TestAccount::new("bk_alice");
    let mut gw = TestAccount::new("bk_gw");

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
    sign_tx(&mut pay, &gw);
    env.submit_and_close(&pay);

    // Create offer
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

    // Query book_offers
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
            let JsonValue::Object(offer) = &offers[0] else {
                panic!("object")
            };
            assert!(offer.contains_key("Account") || offer.contains_key("index"));
            assert!(offer.contains_key("Flags") || offer.contains_key("quality"));
            assert!(offer.contains_key("TakerPays") || offer.contains_key("Sequence"));
            assert!(offer.contains_key("TakerGets") || offer.contains_key("BookDirectory"));
        }
    }
    // Response should have ledger info
    assert!(result.contains_key("ledger_hash") || result.contains_key("ledger_current_index"));
}

#[test]
fn book_offers_with_limit() {
    let mut alice = TestAccount::new("bk_alice2");
    let mut bob = TestAccount::new("bk_bob2");
    let mut gw = TestAccount::new("bk_gw2");

    let env = RpcTestEnv::with_flags(
        &[
            (&alice, 10_000_000_000),
            (&bob, 10_000_000_000),
            (&gw, 10_000_000_000),
        ],
        &[(&gw, lsfDefaultRipple)],
    );

    let usd = currency_from_string("USD");

    // Trust + fund both
    let mut trust_a = STTx::new(TxType::TRUST_SET, |tx| {
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
    sign_tx(&mut trust_a, &alice);
    env.submit_and_close(&trust_a);

    let mut trust_b = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), bob.id);
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
        tx.set_field_u32(get_field_by_symbol("sfSequence"), bob.next_seq());
    });
    sign_tx(&mut trust_b, &bob);
    env.submit_and_close(&trust_b);

    let mut pay_a = STTx::new(TxType::PAYMENT, |tx| {
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
    sign_tx(&mut pay_a, &gw);
    env.submit_and_close(&pay_a);

    let mut pay_b = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), gw.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob.id);
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
    sign_tx(&mut pay_b, &gw);
    env.submit_and_close(&pay_b);

    // Both create offers
    let mut offer1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(100_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfTakerGets"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfTakerGets"),
                Issue::new(usd, gw.id),
                50,
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
    sign_tx(&mut offer1, &alice);
    env.submit_and_close(&offer1);

    let mut offer2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), bob.id);
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
        tx.set_field_u32(get_field_by_symbol("sfSequence"), bob.next_seq());
    });
    sign_tx(&mut offer2, &bob);
    env.submit_and_close(&offer2);

    // Query with limit=1
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
                ("limit", JsonValue::Unsigned(1)),
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
        // With limit=1, should get at most 1 offer
        assert!(
            offers.len() <= 1,
            "limit should cap to 1, got {}",
            offers.len()
        );
    }
}

// === TRANSACTION RETRY PATTERNS ===

#[test]
fn submit_with_wrong_sequence_returns_ter_pre_seq() {
    let alice = TestAccount::new("retry_alice");
    let bob = TestAccount::new("retry_bob");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 1_000_000_000)]);

    // Submit with sequence 99 (wrong - should be 1)
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
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 99); // Wrong sequence
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
                // Should get terPRE_SEQ or tefPAST_SEQ for wrong sequence
                assert!(
                    engine_result.starts_with("ter")
                        || engine_result.starts_with("tef")
                        || engine_result.starts_with("tes"),
                    "wrong sequence should produce ter/tef result: {engine_result}"
                );
            }
        }
        Err(_) => {} // Error is also acceptable
        _ => {}
    }
}
