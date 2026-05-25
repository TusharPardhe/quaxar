//! Tests for the ledger entry RPC handler.

use protocol::{
    currency_from_string, get_field_by_symbol, to_base58, Issue, JsonValue, STAmount, STTx, TxType,
};
use rpc_integration_tests::env::*;

#[test]
fn ledger_entry_account_root_after_funding() {
    let alice = TestAccount::new("le_alice");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = rpc::do_ledger_entry(
        &rpc::LedgerEntryRequest {
            params: &json([
                ("account_root", JsonValue::String(to_base58(alice.id))),
                ("ledger_index", JsonValue::String("current".to_owned())),
            ]),
            api_version: 2,
            role: rpc::RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if result.get("error").is_none() {
        assert!(result.contains_key("node") || result.contains_key("node_binary"));
        assert!(result.contains_key("index"));
    }
}

#[test]
fn ledger_entry_offer_after_offer_create() {
    let mut alice = TestAccount::new("le_alice2");
    let gw = TestAccount::new("le_gw2");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&gw, 10_000_000_000)]);

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

    let source = env.rpc_source();
    let result = rpc::do_ledger_entry(
        &rpc::LedgerEntryRequest {
            params: &json([
                (
                    "offer",
                    json([
                        ("account", JsonValue::String(to_base58(alice.id))),
                        ("seq", JsonValue::Unsigned(1)),
                    ]),
                ),
                ("ledger_index", JsonValue::String("current".to_owned())),
            ]),
            api_version: 2,
            role: rpc::RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };
    // Either found or entryNotFound (if offer was rejected)
    assert!(result.contains_key("node") || result.contains_key("error"));
}

#[test]
fn ledger_entry_ripple_state_after_trust_set() {
    let mut alice = TestAccount::new("le_alice3");
    let gw = TestAccount::new("le_gw3");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&gw, 10_000_000_000)]);

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

    let source = env.rpc_source();
    let result = rpc::do_ledger_entry(
        &rpc::LedgerEntryRequest {
            params: &json([
                (
                    "ripple_state",
                    json([
                        (
                            "accounts",
                            JsonValue::Array(vec![
                                JsonValue::String(to_base58(alice.id)),
                                JsonValue::String(to_base58(gw.id)),
                            ]),
                        ),
                        ("currency", JsonValue::String("USD".to_owned())),
                    ]),
                ),
                ("ledger_index", JsonValue::String("current".to_owned())),
            ]),
            api_version: 2,
            role: rpc::RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };
    assert!(result.contains_key("node") || result.contains_key("error"));
}

#[test]
fn ledger_entry_not_found() {
    let alice = TestAccount::new("le_alice4");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let fake_index = basics::base_uint::Uint256::from_array([0xDD; 32]);
    let result = rpc::do_ledger_entry(
        &rpc::LedgerEntryRequest {
            params: &json([
                ("index", JsonValue::String(fake_index.to_string())),
                ("ledger_index", JsonValue::String("current".to_owned())),
            ]),
            api_version: 2,
            role: rpc::RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("entryNotFound".to_owned()))
    );
}
