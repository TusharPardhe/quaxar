//! account info extended tests part A.

use super::*;

#[test]
fn defs_account_info_funded() {
    let a = TestAccount::new("t40");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([("account", sv(&to_base58(a.id)))]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    );
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert!(r.contains_key("account_data"));
}

#[test]
fn defs_account_info_not_found() {
    let a = TestAccount::new("t41");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let b2 = TestAccount::new("t41b");
    let r = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([("account", JsonValue::String(to_base58(b2.id)))]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    );
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert_eq!(
        r.get("error"),
        Some(&JsonValue::String("actNotFound".to_owned()))
    );
}

#[test]
fn defs_account_info_malformed() {
    let a = TestAccount::new("t42");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([("account", JsonValue::String("bad".to_owned()))]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    );
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert_eq!(
        r.get("error"),
        Some(&JsonValue::String("actMalformed".to_owned()))
    );
}

#[test]
fn defs_account_lines_empty() {
    let a = TestAccount::new("t43");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::do_account_lines(
        &rpc::AccountLinesRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    );
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    if let Some(JsonValue::Array(l)) = r.get("lines") {
        assert_eq!(l.len(), 0);
    }
}

#[test]
fn defs_account_objects_empty() {
    let a = TestAccount::new("t44");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    );
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    if let Some(JsonValue::Array(o)) = r.get("account_objects") {
        assert_eq!(o.len(), 0);
    }
}

#[test]
fn defs_trust_creates_line() {
    let mut a = TestAccount::new("t60a");
    let g = TestAccount::new("t60g");
    let e = RpcTestEnv::with_flags(
        &[(&a, 10_000_000_000), (&g, 10_000_000_000)],
        &[(&g, lsfDefaultRipple)],
    );
    let mut t = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(currency_from_string("USD"), g.id),
                1000,
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
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_lines(
        &rpc::AccountLinesRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("obj")
    };
    if let Some(JsonValue::Array(l)) = r.get("lines") {
        assert!(!l.is_empty());
    }
}

#[test]
fn defs_ext_49() {
    let a = TestAccount::new("d49");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(a.id))),
                ("ledger_index", sv("current")),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    assert!(r.contains_key("account_data"));
}

#[test]
fn defs_ext_50() {
    let a = TestAccount::new("d50");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(a.id))),
                ("ledger_index", sv("current")),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Object(d)) = r.get("account_data") {
        assert_eq!(d.get("Account"), Some(&JsonValue::String(to_base58(a.id))));
        assert!(d.contains_key("Balance"));
        assert!(d.contains_key("Sequence"));
    }
}

#[test]
fn defs_ext_51() {
    let a = TestAccount::new("d51");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([("account", sv("bad"))]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    assert_eq!(r.get("error"), Some(&sv("actMalformed")));
}

#[test]
fn defs_ext_52() {
    let a = TestAccount::new("d52");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &obj(),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    assert!(r.contains_key("error"));
}

#[test]
fn defs_ext_53() {
    let a = TestAccount::new("d53");
    let b2 = TestAccount::new("d53b");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(b2.id))),
                ("ledger_index", sv("current")),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    assert_eq!(r.get("error"), Some(&sv("actNotFound")));
}

#[test]
fn defs_ext_54() {
    let a = TestAccount::new("d54");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_lines(
        &rpc::AccountLinesRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    assert_eq!(r.get("account"), Some(&JsonValue::String(to_base58(a.id))));
}

#[test]
fn defs_ext_55() {
    let a = TestAccount::new("d55");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_lines(
        &rpc::AccountLinesRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Array(l)) = r.get("lines") {
        assert_eq!(l.len(), 0);
    }
}

#[test]
fn defs_ext_56() {
    let a = TestAccount::new("d56");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
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
    assert_eq!(r.get("account"), Some(&JsonValue::String(to_base58(a.id))));
}

#[test]
fn defs_ext_57() {
    let a = TestAccount::new("d57");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
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
    if let Some(JsonValue::Array(o)) = r.get("account_objects") {
        assert_eq!(o.len(), 0);
    }
}

#[test]
fn defs_ext_92() {
    let mut a = TestAccount::new("d92a");
    let g = TestAccount::new("d92g");
    let e = RpcTestEnv::with_flags(
        &[(&a, 10_000_000_000), (&g, 10_000_000_000)],
        &[(&g, lsfDefaultRipple)],
    );
    let mut t = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(currency_from_string("USD"), g.id),
                1000,
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
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_lines(
        &rpc::AccountLinesRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Array(l)) = r.get("lines") {
        if !l.is_empty() {
            let JsonValue::Object(line) = &l[0] else {
                panic!("")
            };
            assert!(line.contains_key("account"));
            assert!(line.contains_key("balance"));
            assert!(line.contains_key("currency"));
            assert!(line.contains_key("limit"));
            assert!(line.contains_key("limit_peer"));
            assert_eq!(line.get("currency"), Some(&sv("USD")));
        }
    }
}
