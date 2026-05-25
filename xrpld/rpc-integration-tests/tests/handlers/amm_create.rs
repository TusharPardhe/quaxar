//! Tests for the amm create RPC handler.

use protocol::{
    currency_from_string, get_field_by_symbol, lsfDefaultRipple, to_base58, Issue, JsonValue,
    STAmount, STTx, TxType,
};
use rpc_integration_tests::env::*;
use rpc_integration_tests::helpers::*;
// AMM with feature enabled via RpcTestEnv
#[test]
fn amm_create_with_feature() {
    let mut a = TestAccount::new("amm1a");
    let mut g = TestAccount::new("amm1g");
    let e = RpcTestEnv::with_flags(
        &[(&a, 100_000_000_000), (&g, 100_000_000_000)],
        &[(&g, lsfDefaultRipple)],
    );
    let usd = currency_from_string("USD");
    let mut t = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(usd, g.id),
                1000000,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
    });
    sign_tx(&mut t, &a);
    e.submit_and_close(&t);
    let mut p = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), g.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), a.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfAmount"),
                Issue::new(usd, g.id),
                100000,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), g.next_seq());
    });
    sign_tx(&mut p, &g);
    e.submit_and_close(&p);
    let mut amm = STTx::new(TxType::AMM_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(10_000_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfAmount2"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfAmount2"),
                Issue::new(usd, g.id),
                10000,
                0,
                false,
            ),
        );
        tx.set_field_u16(get_field_by_symbol("sfTradingFee"), 500);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
    });
    sign_tx(&mut amm, &a);
    e.submit_and_close(&amm);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Array(objs)) = r.get("account_objects") {
        let amm_entry = objs.iter().find(
            |o| matches!(o,JsonValue::Object(obj) if obj.get("LedgerEntryType")==Some(&sv("AMM"))),
        );
        if let Some(JsonValue::Object(amm_obj)) = amm_entry {
            assert_eq!(amm_obj.get("LedgerEntryType"), Some(&sv("AMM")));
            assert!(amm_obj.contains_key("TradingFee") || amm_obj.contains_key("LPTokenBalance"));
            assert!(amm_obj.contains_key("Asset") || amm_obj.contains_key("Asset2"));
        }
    }
}
// ServerDefinitions: AMM format fields
#[test]
fn amm_format_fields() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    if let Some(JsonValue::Array(amm_fields)) = f.get("AMMCreate") {
        let names: Vec<&str> = amm_fields
            .iter()
            .filter_map(|x| {
                if let JsonValue::Object(o) = x {
                    o.get("name").and_then(|n| {
                        if let JsonValue::String(s) = n {
                            Some(s.as_str())
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            })
            .collect();
        assert!(names.contains(&"Amount"));
        assert!(names.contains(&"Amount2"));
        assert!(names.contains(&"TradingFee"));
    }
}
// ServerDefinitions: AMM LE format
#[test]
fn amm_le_format() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    if let Some(JsonValue::Array(amm_fields)) = f.get("AMM") {
        assert!(!amm_fields.is_empty());
    }
}
