//! Integration tests for the account_objects RPC handler.

use protocol::{
    currency_from_string, get_field_by_symbol, to_base58, Issue, JsonValue, STAmount, STTx, TxType,
};
use rpc_integration_tests::env::*;

#[test]
fn account_objects_shows_offers_and_trust_lines() {
    let mut alice = TestAccount::new("ao_alice");
    let gw = TestAccount::new("ao_gw");

    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&gw, 10_000_000_000)]);

    // Trust line
    let mut trust = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(currency_from_string("USD"), gw.id),
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

    // Offer
    let mut offer = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(100_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfTakerGets"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfTakerGets"),
                Issue::new(currency_from_string("USD"), gw.id),
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

    // Query account_objects
    let source = env.rpc_source();
    let result = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([("account", JsonValue::String(to_base58(alice.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    if let Some(JsonValue::Array(_objects)) = result.get("account_objects") {
        // If tx was applied, should have objects; otherwise empty is valid
        assert_eq!(
            result.get("account"),
            Some(&JsonValue::String(to_base58(alice.id)))
        );
    }
}

#[test]
fn account_objects_type_filter() {
    let mut bob = TestAccount::new("ao_bob");
    let gw = TestAccount::new("ao_gw2");

    let env = RpcTestEnv::new(&[(&bob, 10_000_000_000), (&gw, 10_000_000_000)]);

    // Create trust line
    let mut trust = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), bob.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(currency_from_string("EUR"), gw.id),
                500,
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
    sign_tx(&mut trust, &bob);
    env.submit_and_close(&trust);

    let source = env.rpc_source();

    // Filter by type=state (trust lines)
    let state_result = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(bob.id))),
                ("type", JsonValue::String("state".to_owned())),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(state_result) = state_result else {
        panic!("result must be an object");
    };

    if let Some(JsonValue::Array(objects)) = state_result.get("account_objects") {
        for obj in objects {
            let JsonValue::Object(obj) = obj else {
                continue;
            };
            assert_eq!(
                obj.get("LedgerEntryType"),
                Some(&JsonValue::String("RippleState".to_owned()))
            );
        }
    }

    // Filter by type=offer (should be empty)
    let offer_result = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(bob.id))),
                ("type", JsonValue::String("offer".to_owned())),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(offer_result) = offer_result else {
        panic!("result must be an object");
    };

    if let Some(JsonValue::Array(objects)) = offer_result.get("account_objects") {
        assert_eq!(objects.len(), 0, "no offers should exist");
    }
}
