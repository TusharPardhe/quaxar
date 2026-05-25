//! Tests for the no ripple check RPC handler.

use protocol::{
    currency_from_string, get_field_by_symbol, to_base58, Issue, JsonValue, STAmount, STTx, TxType,
};
use rpc_integration_tests::env::*;

#[test]
fn no_ripple_check_gateway_without_default_ripple() {
    let gw = TestAccount::new("nrc_gw");
    let env = RpcTestEnv::new(&[(&gw, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = rpc::do_no_ripple_check(
        &rpc::NoRippleCheckRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(gw.id))),
                ("role", JsonValue::String("gateway".to_owned())),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if result.get("error").is_none() {
        // Gateway without defaultRipple should have problems
        assert!(result.contains_key("problems"));
        let JsonValue::Array(problems) = result.get("problems").unwrap() else {
            panic!("array")
        };
        assert!(
            !problems.is_empty(),
            "gateway without defaultRipple should have problems"
        );
    }
}

#[test]
fn no_ripple_check_user_role() {
    let mut alice = TestAccount::new("nrc_alice");
    let gw = TestAccount::new("nrc_gw2");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&gw, 10_000_000_000)]);

    // Create trust line
    let mut trust = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(currency_from_string("USD"), gw.id),
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

    let source = env.rpc_source();
    let result = rpc::do_no_ripple_check(
        &rpc::NoRippleCheckRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(alice.id))),
                ("role", JsonValue::String("user".to_owned())),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if result.get("error").is_none() {
        assert!(result.contains_key("problems"));
    }
}

#[test]
fn ledger_data_returns_state_entries() {
    let alice = TestAccount::new("ld_alice");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = rpc::do_ledger_data(
        &rpc::LedgerDataRequest {
            params: &json([("ledger_index", JsonValue::String("current".to_owned()))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if result.get("error").is_none() {
        assert!(result.contains_key("state"));
        let JsonValue::Array(state) = result.get("state").unwrap() else {
            panic!("array")
        };
        // Should have at least alice's account root
        assert!(!state.is_empty(), "should have state entries");
    }
}

#[test]
fn ledger_data_binary_mode() {
    let alice = TestAccount::new("ld_alice2");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = rpc::do_ledger_data(
        &rpc::LedgerDataRequest {
            params: &json([
                ("ledger_index", JsonValue::String("current".to_owned())),
                ("binary", JsonValue::Bool(true)),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if result.get("error").is_none() {
        assert!(result.contains_key("state"));
    }
}
