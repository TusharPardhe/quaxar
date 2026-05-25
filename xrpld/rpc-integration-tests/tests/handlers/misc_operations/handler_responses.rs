//! misc operations tests part A.

use super::*;

#[test]
fn defs_account_offers_empty() {
    let a = TestAccount::new("t45");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::do_account_offers(
        &rpc::AccountOffersRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    );
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    if let Some(JsonValue::Array(o)) = r.get("offers") {
        assert_eq!(o.len(), 0);
    }
}

#[test]
fn defs_ext_58() {
    let a = TestAccount::new("d58");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_offers(
        &rpc::AccountOffersRequest {
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
fn defs_ext_59() {
    let a = TestAccount::new("d59");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_offers(
        &rpc::AccountOffersRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Array(o)) = r.get("offers") {
        assert_eq!(o.len(), 0);
    }
}

#[test]
fn defs_fmt_71() {
    let a = TestAccount::new("c71");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_offers(
        &rpc::AccountOffersRequest {
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
fn defs_fmt_93() {
    let a = TestAccount::new("c93");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_offers(
        &rpc::AccountOffersRequest {
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
fn defs_deposit_auth_self() {
    let a = TestAccount::new("t49");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::do_deposit_authorized(
        &rpc::DepositAuthorizedRequest {
            params: &json([
                ("source_account", JsonValue::String(to_base58(a.id))),
                ("destination_account", JsonValue::String(to_base58(a.id))),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    );
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    if r.get("error").is_none() {
        assert_eq!(r.get("deposit_authorized"), Some(&b(true)));
    }
}

#[test]
fn defs_ext_64() {
    let a = TestAccount::new("d64");
    let b2 = TestAccount::new("d64b");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000), (&b2, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_deposit_authorized(
        &rpc::DepositAuthorizedRequest {
            params: &json([
                ("source_account", JsonValue::String(to_base58(a.id))),
                ("destination_account", JsonValue::String(to_base58(b2.id))),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert_eq!(r.get("deposit_authorized"), Some(&b(true)));
        assert_eq!(
            r.get("source_account"),
            Some(&JsonValue::String(to_base58(a.id)))
        );
        assert_eq!(
            r.get("destination_account"),
            Some(&JsonValue::String(to_base58(b2.id)))
        );
    }
}

#[test]
fn defs_ext_65() {
    let a = TestAccount::new("d65");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_deposit_authorized(
        &rpc::DepositAuthorizedRequest {
            params: &json([
                ("source_account", JsonValue::String(to_base58(a.id))),
                ("destination_account", JsonValue::String(to_base58(a.id))),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert_eq!(r.get("deposit_authorized"), Some(&b(true)));
    }
}

#[test]
fn defs_ext_66() {
    let a = TestAccount::new("d66");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_deposit_authorized(
        &rpc::DepositAuthorizedRequest {
            params: &obj(),
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
fn defs_fmt_72() {
    let a = TestAccount::new("c72");
    let b2 = TestAccount::new("c72b");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000), (&b2, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_deposit_authorized(
        &rpc::DepositAuthorizedRequest {
            params: &json([
                ("source_account", JsonValue::String(to_base58(a.id))),
                ("destination_account", JsonValue::String(to_base58(b2.id))),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert_eq!(r.get("deposit_authorized"), Some(&b(true)));
    }
}

#[test]
fn defs_fmt_98() {
    let a = TestAccount::new("c98");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_deposit_authorized(
        &rpc::DepositAuthorizedRequest {
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
fn defs_types_100_deposit_auth_missing_src() {
    let a = TestAccount::new("b100");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_deposit_authorized(
        &rpc::DepositAuthorizedRequest {
            params: &json([("destination_account", JsonValue::String(to_base58(a.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("obj")
    };
    assert!(r.contains_key("error"));
}
#[test]
fn defs_no_ripple_check_missing() {
    let a = TestAccount::new("t50");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::do_no_ripple_check(
        &rpc::NoRippleCheckRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(a.id))),
                ("role", JsonValue::String("gateway".to_owned())),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    );
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    if r.get("error").is_none() {
        assert!(r.contains_key("problems"));
    }
}

#[test]
fn defs_ext_67() {
    let a = TestAccount::new("d67");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_no_ripple_check(
        &rpc::NoRippleCheckRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(a.id))),
                ("role", sv("gateway")),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert!(r.contains_key("problems"));
    }
}

#[test]
fn defs_ext_68() {
    let a = TestAccount::new("d68");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_no_ripple_check(
        &rpc::NoRippleCheckRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(a.id))),
                ("role", sv("user")),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert!(r.contains_key("problems"));
    }
}

#[test]
fn defs_fmt_73() {
    let a = TestAccount::new("c73");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_no_ripple_check(
        &rpc::NoRippleCheckRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(a.id))),
                ("role", sv("gateway")),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert!(r.contains_key("problems"));
    }
}

#[test]
fn defs_fmt_74() {
    let a = TestAccount::new("c74");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_no_ripple_check(
        &rpc::NoRippleCheckRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(a.id))),
                ("role", sv("user")),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert!(r.contains_key("problems"));
    }
}

#[test]
fn defs_ext_69() {
    let a = TestAccount::new("d69");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_gateway_balances(
        &rpc::GatewayBalancesRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 2,
            role: rpc::RpcRole::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert_eq!(r.get("account"), Some(&JsonValue::String(to_base58(a.id))));
    }
}

#[test]
fn defs_ext_70() {
    let a = TestAccount::new("d70");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger(
        &json([("ledger_index", sv("current")), ("full", b(true))]),
        rpc::RpcRole::Guest,
        2,
        &s,
    ) else {
        panic!("")
    };
    assert_eq!(r.get("error"), Some(&sv("noPermission")));
}

#[test]
fn defs_ext_71() {
    let a = TestAccount::new("d71");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger(
        &json([("ledger_index", sv("current")), ("accounts", b(true))]),
        rpc::RpcRole::Guest,
        2,
        &s,
    ) else {
        panic!("")
    };
    assert_eq!(r.get("error"), Some(&sv("noPermission")));
}

#[test]
fn defs_ext_72() {
    let a = TestAccount::new("d72");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger(
        &json([("ledger_index", sv("validated")), ("queue", b(true))]),
        rpc::RpcRole::Admin,
        2,
        &s,
    ) else {
        panic!("")
    };
    assert_eq!(r.get("error"), Some(&sv("invalidParams")));
}

#[test]
fn defs_ext_73() {
    let a = TestAccount::new("d73");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger(
        &json([("ledger_index", u(99999))]),
        rpc::RpcRole::Admin,
        2,
        &s,
    ) else {
        panic!("")
    };
    assert_eq!(r.get("error"), Some(&sv("lgrNotFound")));
}
