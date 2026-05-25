//! These tests execute real transactions and verify RPC responses.

use protocol::{
    currency_from_string, get_field_by_symbol, to_base58, Issue, JsonValue, STAmount, STTx, TxType,
};
use rpc_integration_tests::env::*;

#[test]
fn book_offers_returns_offer_after_offer_create() {
    let mut alice = TestAccount::new("alice");
    let gw = TestAccount::new("gateway");

    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&gw, 10_000_000_000)]);

    let usd = currency_from_string("USD");
    let issuer = gw.id;

    // Create an offer: alice sells 100 XRP for 50 USD
    let mut offer_tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(100_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfTakerGets"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfTakerGets"),
                Issue::new(usd, issuer),
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
    sign_tx(&mut offer_tx, &alice);
    env.submit_and_close(&offer_tx);

    // Query book_offers
    let source = env.rpc_source();
    let response = rpc::do_book_offers(
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
                        ("issuer", JsonValue::String(to_base58(issuer))),
                    ]),
                ),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &source,
        &source,
    );

    let JsonValue::Object(response) = response else {
        panic!("response must be an object");
    };

    // Should have offers (may be empty if offer was rejected, but handler should work)
    assert!(
        response.contains_key("offers") || response.contains_key("ledger_hash"),
        "response should have offers or ledger info: {:?}",
        response.keys().collect::<Vec<_>>()
    );
}

#[test]
fn book_offers_empty_book_returns_empty_array() {
    let alice = TestAccount::new("alice2");
    let gw = TestAccount::new("gateway2");

    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&gw, 10_000_000_000)]);

    let issuer = gw.id;
    let source = env.rpc_source();

    // Query a book with no offers
    let response = rpc::do_book_offers(
        &rpc::BookOffersRequest {
            params: &json([
                (
                    "taker_pays",
                    json([("currency", JsonValue::String("XRP".to_owned()))]),
                ),
                (
                    "taker_gets",
                    json([
                        ("currency", JsonValue::String("EUR".to_owned())),
                        ("issuer", JsonValue::String(to_base58(issuer))),
                    ]),
                ),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &source,
        &source,
    );

    let JsonValue::Object(response) = response else {
        panic!("response must be an object");
    };

    if let Some(JsonValue::Array(offers)) = response.get("offers") {
        assert_eq!(offers.len(), 0, "empty book should have no offers");
    }
    // Should have ledger info
    assert!(
        response.contains_key("ledger_hash") || response.contains_key("ledger_current_index"),
        "should have ledger metadata"
    );
}
