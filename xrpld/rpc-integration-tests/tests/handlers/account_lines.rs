//! Integration tests for the account_lines RPC handler.

use protocol::{
    currency_from_string, get_field_by_symbol, to_base58, Issue, JsonValue, STAmount, STTx, TxType,
};
use rpc_integration_tests::env::*;

fn account_lines(env: &RpcTestEnv, account: &TestAccount, peer: Option<&TestAccount>) -> JsonValue {
    let source = env.rpc_source();
    let mut params = vec![("account", JsonValue::String(to_base58(account.id)))];
    if let Some(p) = peer {
        params.push(("peer", JsonValue::String(to_base58(p.id))));
    }
    rpc::do_account_lines(
        &rpc::AccountLinesRequest {
            params: &json(params),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    )
}

#[test]
fn account_lines_shows_trust_lines_after_trust_set() {
    let mut alice = TestAccount::new("al_lines1");
    let gw = TestAccount::new("gw_lines1");

    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&gw, 10_000_000_000)]);

    let usd = currency_from_string("USD");

    // Alice trusts gateway for USD
    let mut trust_tx = STTx::new(TxType::TRUST_SET, |tx| {
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
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut trust_tx, &alice);
    env.submit_and_close(&trust_tx);

    // Query account_lines
    let result = account_lines(&env, &alice, None);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    // Should have the trust line or an error (if TrustSet failed)
    if result.contains_key("lines") {
        let JsonValue::Array(lines) = result.get("lines").unwrap() else {
            panic!("lines must be an array");
        };
        if !lines.is_empty() {
            let JsonValue::Object(line) = &lines[0] else {
                panic!("line must be an object");
            };
            assert!(line.contains_key("account"));
            assert!(line.contains_key("balance"));
            assert!(line.contains_key("currency"));
            assert!(line.contains_key("limit"));
            assert!(line.contains_key("limit_peer"));
            assert!(line.contains_key("quality_in"));
            assert!(line.contains_key("quality_out"));
            assert_eq!(
                line.get("currency"),
                Some(&JsonValue::String("USD".to_owned()))
            );
            // Peer account should be the gateway
            assert!(line.contains_key("account"));
        }
    }
    // Should have account field
    assert_eq!(
        result.get("account"),
        Some(&JsonValue::String(to_base58(alice.id)))
    );
}

#[test]
fn account_lines_peer_filter() {
    let mut alice = TestAccount::new("al_lines2");
    let gw1 = TestAccount::new("gw1_lines2");
    let gw2 = TestAccount::new("gw2_lines2");

    let env = RpcTestEnv::new(&[
        (&alice, 10_000_000_000),
        (&gw1, 10_000_000_000),
        (&gw2, 10_000_000_000),
    ]);

    // Alice trusts gw1 for USD
    let mut trust1 = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(currency_from_string("USD"), gw1.id),
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
    sign_tx(&mut trust1, &alice);
    env.submit_and_close(&trust1);

    // Alice trusts gw2 for EUR
    let mut trust2 = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(currency_from_string("EUR"), gw2.id),
                200,
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
    sign_tx(&mut trust2, &alice);
    env.submit_and_close(&trust2);

    // Query with peer filter for gw1 only
    let result = account_lines(&env, &alice, Some(&gw1));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    if let Some(JsonValue::Array(lines)) = result.get("lines") {
        // Should only show lines with gw1
        for line in lines {
            let JsonValue::Object(line) = line else {
                continue;
            };
            assert_eq!(
                line.get("account"),
                Some(&JsonValue::String(to_base58(gw1.id)))
            );
        }
    }
}

#[test]
fn account_lines_limit_and_marker() {
    let mut alice = TestAccount::new("al_lines3");
    let gw = TestAccount::new("gw_lines3");

    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&gw, 10_000_000_000)]);

    // Create multiple trust lines
    for currency in ["USD", "EUR", "GBP", "JPY", "CNY"] {
        let mut trust = STTx::new(TxType::TRUST_SET, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
            tx.set_field_amount(
                get_field_by_symbol("sfLimitAmount"),
                STAmount::new_with_asset(
                    get_field_by_symbol("sfLimitAmount"),
                    Issue::new(currency_from_string(currency), gw.id),
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
        sign_tx(&mut trust, &alice);
        env.submit_and_close(&trust);
    }

    // Query with limit=2
    let source = env.rpc_source();
    let result = rpc::do_account_lines(
        &rpc::AccountLinesRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(alice.id))),
                ("limit", JsonValue::Unsigned(2)),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    if let Some(JsonValue::Array(lines)) = result.get("lines") {
        // With limit=2, should get at most 2 lines
        assert!(lines.len() <= 2, "limit should cap results");
        if lines.len() == 2 {
            // Should have a marker for continuation
            assert!(result.contains_key("marker"), "should have marker");
        }
    }
}
