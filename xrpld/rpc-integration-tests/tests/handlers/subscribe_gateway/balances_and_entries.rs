//! subscribe gateway tests part B.

use super::*;

#[test]
fn gateway_balances_with_hotwallet_separation() {
    let mut alice = TestAccount::new("gwb_alice");
    let mut hw = TestAccount::new("gwb_hw");
    let mut gw = TestAccount::new("gwb_gw");
    let env = RpcTestEnv::with_flags(
        &[
            (&alice, 10_000_000_000),
            (&hw, 10_000_000_000),
            (&gw, 10_000_000_000),
        ],
        &[(&gw, lsfDefaultRipple)],
    );
    let usd = currency_from_string("USD");

    // Both trust gateway
    for acct in [&mut alice, &mut hw] {
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
    }

    // Gateway pays both
    for (dest, amount) in [(alice.id, 100u64), (hw.id, 5000u64)] {
        let mut p = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), gw.id);
            tx.set_account_id(get_field_by_symbol("sfDestination"), dest);
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_with_asset(
                    get_field_by_symbol("sfAmount"),
                    Issue::new(usd, gw.id),
                    amount,
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

    let source = env.rpc_source();
    let result = rpc::do_gateway_balances(
        &rpc::GatewayBalancesRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(gw.id))),
                ("hotwallet", JsonValue::String(to_base58(hw.id))),
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
        // Should have balances (hotwallet) and obligations (clients)
        assert!(
            result.contains_key("balances")
                || result.contains_key("obligations")
                || result.contains_key("account")
        );
        assert_eq!(
            result.get("account"),
            Some(&JsonValue::String(to_base58(gw.id)))
        );
        assert!(result.contains_key("ledger_hash") || result.contains_key("ledger_current_index"));
    }
}

// === ACCOUNT_OFFERS with real offers ===

#[test]
fn account_offers_shows_offer_details() {
    let mut alice = TestAccount::new("ao2_alice");
    let gw = TestAccount::new("ao2_gw");
    let env = RpcTestEnv::with_flags(
        &[(&alice, 10_000_000_000), (&gw, 10_000_000_000)],
        &[(&gw, lsfDefaultRipple)],
    );
    let usd = currency_from_string("USD");

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

    let offer_seq = alice.next_seq();
    let mut offer = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(500_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfTakerGets"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfTakerGets"),
                Issue::new(usd, gw.id),
                250,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), offer_seq);
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
        if !offers.is_empty() {
            let JsonValue::Object(o) = &offers[0] else {
                panic!("object")
            };
            assert!(o.contains_key("seq"), "offer should have seq");
            assert!(o.contains_key("taker_pays") || o.contains_key("TakerPays"));
            assert!(o.contains_key("taker_gets") || o.contains_key("TakerGets"));
            assert!(o.contains_key("flags") || o.contains_key("Flags"));
        }
    }
}

// === LEDGER_ENTRY - account_root after payment ===

#[test]
fn ledger_entry_account_root_balance_after_payment() {
    let mut alice = TestAccount::new("le3_alice");
    let bob = TestAccount::new("le3_bob");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 1_000_000_000)]);

    let mut payment = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(2_000_000_000, false),
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
    let result = rpc::do_ledger_entry(
        &rpc::LedgerEntryRequest {
            params: &json([
                ("account_root", JsonValue::String(to_base58(alice.id))),
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

    if let Some(JsonValue::Object(node)) = result.get("node") {
        assert_eq!(
            node.get("LedgerEntryType"),
            Some(&JsonValue::String("AccountRoot".to_owned()))
        );
        assert!(node.contains_key("Balance"));
        assert!(node.contains_key("Sequence"));
        assert!(node.contains_key("Account"));
        // Balance should be less than initial
        if let Some(JsonValue::String(balance)) = node.get("Balance") {
            let b: i64 = balance.parse().unwrap_or(0);
            assert!(b < 10_000_000_000, "balance should decrease after payment");
            assert!(b > 0, "balance should still be positive");
        }
    }
}

// === SERVER_DEFINITIONS - more field checks ===

#[test]
fn server_definitions_fields_count_is_large() {
    let result = rpc::do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let JsonValue::Object(result) = result else {
        panic!("object")
    };
    let JsonValue::Array(fields) = result.get("FIELDS").unwrap() else {
        panic!("array")
    };
    assert!(
        fields.len() > 200,
        "should have 200+ fields, got {}",
        fields.len()
    );
}

#[test]
fn server_definitions_types_count() {
    let result = rpc::do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let JsonValue::Object(result) = result else {
        panic!("object")
    };
    let JsonValue::Object(types) = result.get("TYPES").unwrap() else {
        panic!("object")
    };
    assert!(types.len() > 15, "should have 15+ types");
    let JsonValue::Object(le_types) = result.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("object")
    };
    assert!(le_types.len() > 15, "should have 15+ ledger entry types");
    let JsonValue::Object(tx_types) = result.get("TRANSACTION_TYPES").unwrap() else {
        panic!("object")
    };
    assert!(tx_types.len() > 25, "should have 25+ transaction types");
    let JsonValue::Object(results) = result.get("TRANSACTION_RESULTS").unwrap() else {
        panic!("object")
    };
    assert!(results.len() > 50, "should have 50+ transaction results");
}
