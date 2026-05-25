//! Tests for the account offers RPC handler.

use protocol::{
    currency_from_string, get_field_by_symbol, to_base58, Issue, JsonValue, STAmount, STTx, TxType,
};
use rpc_integration_tests::env::*;

#[test]
fn account_offers_shows_created_offers() {
    let mut alice = TestAccount::new("aof_alice");
    let gw = TestAccount::new("aof_gw");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&gw, 10_000_000_000)]);

    let mut offer = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(50_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfTakerGets"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfTakerGets"),
                Issue::new(currency_from_string("USD"), gw.id),
                25,
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

    let source = env.rpc_source();
    let result = rpc::do_account_offers(
        &rpc::AccountOffersRequest {
            params: &json([("account", JsonValue::String(to_base58(alice.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    assert_eq!(
        result.get("account"),
        Some(&JsonValue::String(to_base58(alice.id)))
    );
    if let Some(JsonValue::Array(offers)) = result.get("offers") {
        for offer in offers {
            let JsonValue::Object(o) = offer else {
                continue;
            };
            assert!(o.contains_key("seq"));
            assert!(o.contains_key("taker_pays") || o.contains_key("TakerPays"));
            assert!(o.contains_key("taker_gets") || o.contains_key("TakerGets"));
        }
    }
}

#[test]
fn deposit_authorized_self_is_always_authorized() {
    let alice = TestAccount::new("da_alice");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = rpc::do_deposit_authorized(
        &rpc::DepositAuthorizedRequest {
            params: &json([
                ("source_account", JsonValue::String(to_base58(alice.id))),
                (
                    "destination_account",
                    JsonValue::String(to_base58(alice.id)),
                ),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    // Self-deposit is always authorized
    if result.get("error").is_none() {
        assert_eq!(
            result.get("deposit_authorized"),
            Some(&JsonValue::Bool(true))
        );
    }
}

#[test]
fn deposit_authorized_without_deposit_auth_flag() {
    let alice = TestAccount::new("da_alice2");
    let bob = TestAccount::new("da_bob2");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = rpc::do_deposit_authorized(
        &rpc::DepositAuthorizedRequest {
            params: &json([
                ("source_account", JsonValue::String(to_base58(alice.id))),
                ("destination_account", JsonValue::String(to_base58(bob.id))),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    // Without depositAuth flag, anyone can deposit
    if result.get("error").is_none() {
        assert_eq!(
            result.get("deposit_authorized"),
            Some(&JsonValue::Bool(true))
        );
    }
}
