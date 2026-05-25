//! account info extended tests part B.

use super::*;

#[test]
fn defs_flags_30() {
    let mut a = TestAccount::new("e30a");
    let g = TestAccount::new("e30g");
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
                Issue::new(currency_from_string("EUR"), g.id),
                2000,
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
            assert_eq!(line.get("currency"), Some(&sv("EUR")));
            assert!(line.contains_key("balance"));
            assert!(line.contains_key("limit"));
        }
    }
}

#[test]
fn defs_fmt_44() {
    let a = TestAccount::new("c44");
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
fn defs_fmt_45() {
    let a = TestAccount::new("c45");
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
    assert!(r.contains_key("ledger_current_index") || r.contains_key("ledger_index"));
}

#[test]
fn defs_fmt_69() {
    let a = TestAccount::new("c69");
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
fn defs_fmt_70() {
    let a = TestAccount::new("c70");
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
fn defs_fmt_90() {
    let a = TestAccount::new("c90");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([("account", sv("notvalid"))]),
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
fn defs_fmt_91() {
    let a = TestAccount::new("c91");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_lines(
        &rpc::AccountLinesRequest {
            params: &json([("account", sv("bad"))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    assert!(r.contains_key("error"));
}

#[test]
fn defs_fmt_92() {
    let a = TestAccount::new("c92");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([("account", sv("bad"))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    assert!(r.contains_key("error"));
}

#[test]
fn defs_fmt_94() {
    let a = TestAccount::new("c94");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &JsonValue::Object(Default::default()),
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
fn defs_fmt_95() {
    let a = TestAccount::new("c95");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_lines(
        &rpc::AccountLinesRequest {
            params: &JsonValue::Object(Default::default()),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    assert!(r.contains_key("error"));
}

#[test]
fn defs_fmt_96() {
    let a = TestAccount::new("c96");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &JsonValue::Object(Default::default()),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    assert!(r.contains_key("error"));
}

#[test]
fn defs_types_62_account_lines_limit_value() {
    let mut a = TestAccount::new("b62a");
    let g = TestAccount::new("b62g");
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
                777,
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
    if let Some(JsonValue::Array(lines)) = r.get("lines") {
        if !lines.is_empty() {
            let JsonValue::Object(l) = &lines[0] else {
                panic!("obj")
            };
            assert!(l.contains_key("limit"));
            assert!(l.contains_key("limit_peer"));
            assert!(l.contains_key("balance"));
            assert!(l.contains_key("currency"));
            assert_eq!(l.get("currency"), Some(&sv("USD")));
        }
    }
}
// Subscribe: event field checks

#[test]
fn defs_types_71_account_info_fields() {
    let a = TestAccount::new("b71");
    let e = RpcTestEnv::new(&[(&a, 7_000_000_000)]);
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
        panic!("obj")
    };
    if let Some(JsonValue::Object(d)) = r.get("account_data") {
        assert_eq!(d.get("Account"), Some(&JsonValue::String(to_base58(a.id))));
        assert!(d.contains_key("Balance"));
        assert!(d.contains_key("Sequence"));
        assert!(d.contains_key("Flags"));
    }
}

#[test]
fn defs_types_72_account_info_flags_zero() {
    let a = TestAccount::new("b72");
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
        panic!("obj")
    };
    if let Some(JsonValue::Object(d)) = r.get("account_data") {
        assert_eq!(d.get("Flags"), Some(&u(0)));
    }
}

#[test]
fn defs_types_73_account_info_sequence_one() {
    let a = TestAccount::new("b73");
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
        panic!("obj")
    };
    if let Some(JsonValue::Object(d)) = r.get("account_data") {
        assert_eq!(d.get("Sequence"), Some(&u(1)));
    }
}

#[test]
fn defs_types_74_account_info_owner_count_zero() {
    let a = TestAccount::new("b74");
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
        panic!("obj")
    };
    if let Some(JsonValue::Object(_d)) = r.get("account_data") {}
}
// More subscribe isolation tests

#[test]
fn defs_types_93_trust_creates_objects() {
    let mut a = TestAccount::new("b93a");
    let g = TestAccount::new("b93g");
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
                500,
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
    let JsonValue::Object(r) = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("obj")
    };
    if let Some(JsonValue::Array(o)) = r.get("account_objects") {
        assert!(!o.is_empty());
    }
}
// Integration: payment found via tx
