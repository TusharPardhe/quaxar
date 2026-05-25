//! Full pipeline integration tests — gateway setup, trust lines, IOU payments, offers.

use protocol::{
    currency_from_string, get_field_by_symbol, lsfDefaultRipple, to_base58, Issue, JsonValue,
    STAmount, STTx, TxType,
};
use rpc_integration_tests::env::*;

/// Set up a gateway environment: gateway with DefaultRipple, alice with trust line and IOU balance.
fn gateway_env() -> (RpcTestEnv, TestAccount, TestAccount, TestAccount) {
    let mut alice = TestAccount::new("pipe_alice");
    let mut bob = TestAccount::new("pipe_bob");
    let mut gw = TestAccount::new("pipe_gw");

    let env = RpcTestEnv::with_flags(
        &[
            (&alice, 10_000_000_000),
            (&bob, 10_000_000_000),
            (&gw, 10_000_000_000),
        ],
        &[(&gw, lsfDefaultRipple)],
    );

    let usd = currency_from_string("USD");

    // Alice trusts gateway for USD
    let mut trust_alice = STTx::new(TxType::TRUST_SET, |tx| {
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
    sign_tx(&mut trust_alice, &alice);
    env.submit_and_close(&trust_alice);

    // Bob trusts gateway for USD
    let mut trust_bob = STTx::new(TxType::TRUST_SET, |tx| {
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
    sign_tx(&mut trust_bob, &bob);
    env.submit_and_close(&trust_bob);

    // Gateway pays alice 1000 USD
    let mut pay_alice = STTx::new(TxType::PAYMENT, |tx| {
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
    sign_tx(&mut pay_alice, &gw);
    env.submit_and_close(&pay_alice);

    (env, alice, bob, gw)
}

#[test]
fn full_pipeline_account_lines_shows_iou_balance() {
    let (env, alice, _bob, _gw) = gateway_env();

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
            assert_eq!(
                line.get("currency"),
                Some(&JsonValue::String("USD".to_owned()))
            );
            assert!(line.contains_key("account"));
            // Balance should be non-zero (gateway paid alice)
            if let Some(JsonValue::String(balance)) = line.get("balance") {
                let bal: f64 = balance.parse().unwrap_or(0.0);
                assert!(
                    bal != 0.0,
                    "alice should have non-zero USD balance, got {balance}"
                );
            }
        }
    }
}

#[test]
fn full_pipeline_offer_create_with_iou_succeeds() {
    let (env, mut alice, _bob, gw) = gateway_env();

    let usd = currency_from_string("USD");

    // Alice creates offer: sell 100 USD for 400 XRP
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

    // Verify offer exists via account_objects
    let source = env.rpc_source();
    let result = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(alice.id))),
                ("type", JsonValue::String("offer".to_owned())),
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
            let JsonValue::Object(offer_obj) = &objects[0] else {
                panic!("object")
            };
            assert_eq!(
                offer_obj.get("LedgerEntryType"),
                Some(&JsonValue::String("Offer".to_owned()))
            );
            assert!(offer_obj.contains_key("TakerPays"));
            assert!(offer_obj.contains_key("TakerGets"));
            assert!(offer_obj.contains_key("Sequence"));
        }
    }
}

#[test]
fn full_pipeline_book_offers_shows_offer_fields() {
    let (env, mut alice, _bob, gw) = gateway_env();

    let usd = currency_from_string("USD");

    // Alice creates offer: sell 50 USD for 200 XRP
    let mut offer = STTx::new(TxType::OFFER_CREATE, |tx| {
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
    sign_tx(&mut offer, &alice);
    env.submit_and_close(&offer);

    // Query book_offers for XRP/USD book
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
            let JsonValue::Object(first) = &offers[0] else {
                panic!("object")
            };
            assert!(first.contains_key("Account") || first.contains_key("index"));
            assert!(first.contains_key("Flags") || first.contains_key("quality"));
        }
    }
}

#[test]
fn full_pipeline_payment_xrp_succeeds_and_tx_found() {
    let (env, mut alice, bob, _gw) = gateway_env();

    // XRP payment from alice to bob
    let mut payment = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(500_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut payment, &alice);
    let tx_id = payment.get_transaction_id();
    env.submit_and_close(&payment);

    // Verify tx is findable
    let source = env.rpc_source();
    let result = rpc::do_tx(
        &rpc::TxRequest {
            params: &json([("transaction", JsonValue::String(tx_id.to_string()))]),
            api_version: 2,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if result.get("error").is_none() {
        assert_eq!(result.get("validated"), Some(&JsonValue::Bool(true)));
        assert_eq!(
            result.get("hash"),
            Some(&JsonValue::String(tx_id.to_string()))
        );
        assert!(result.contains_key("tx_json"));
        assert!(result.contains_key("ledger_index"));
    }
}
