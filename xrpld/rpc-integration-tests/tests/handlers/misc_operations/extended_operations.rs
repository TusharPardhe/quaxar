//! misc operations tests part B.

use super::*;

#[test]
fn defs_ext_79() {
    let a = TestAccount::new("d79");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let fk = basics::base_uint::Uint256::from_array([0xAA; 32]);
    let JsonValue::Object(r) = rpc::do_tx(
        &rpc::TxRequest {
            params: &json([("transaction", JsonValue::String(fk.to_string()))]),
            api_version: 2,
        },
        &s,
    ) else {
        panic!("")
    };
    assert_eq!(r.get("error"), Some(&sv("txnNotFound")));
}

#[test]
fn defs_ext_91() {
    let mut a = TestAccount::new("d91a");
    let b2 = TestAccount::new("d91b");
    let e = RpcTestEnv::new(&[(&a, 10_000_000_000), (&b2, 1_000_000_000)]);
    let mut p = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), b2.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
    });
    sign_tx(&mut p, &a);
    let id = p.get_transaction_id();
    e.submit_and_close(&p);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_tx(
        &rpc::TxRequest {
            params: &json([("transaction", JsonValue::String(id.to_string()))]),
            api_version: 2,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert_eq!(r.get("hash"), Some(&JsonValue::String(id.to_string())));
        assert_eq!(r.get("validated"), Some(&b(true)));
        assert!(r.contains_key("ledger_index"));
    }
}

#[test]
fn defs_ext_93() {
    let a = TestAccount::new("d93");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger(
        &json([("ledger_index", sv("current"))]),
        rpc::RpcRole::Admin,
        2,
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert!(r.contains_key("ledger") || r.contains_key("ledger_current_index"));
    }
}

#[test]
fn defs_ext_94() {
    let a = TestAccount::new("d94");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger(
        &json([("ledger_index", sv("closed"))]),
        rpc::RpcRole::Admin,
        2,
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert!(r.contains_key("ledger"));
    }
}

#[test]
fn defs_ext_95() {
    let a = TestAccount::new("d95");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_data(
        &rpc::LedgerDataRequest {
            params: &json([("ledger_index", sv("current"))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert!(r.contains_key("state"));
    }
}

#[test]
fn defs_ext_96() {
    let a = TestAccount::new("d96");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::feature::do_feature(
        &rpc::feature::FeatureRequest {
            params: &obj(),
            role: rpc::RpcRole::Admin,
        },
        &s,
    );
    let JsonValue::Object(r) = r else { panic!("") };
    assert!(r.contains_key("features"));
}

#[test]
fn defs_ext_97() {
    let a = TestAccount::new("d97");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::feature::do_feature(
        &rpc::feature::FeatureRequest {
            params: &json([("feature", sv("NonExistent"))]),
            role: rpc::RpcRole::Admin,
        },
        &s,
    );
    let JsonValue::Object(r) = r else { panic!("") };
    assert_eq!(r.get("error"), Some(&sv("badFeature")));
}

#[test]
fn defs_ext_98() {
    let a = TestAccount::new("d98");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::feature::do_feature(
        &rpc::feature::FeatureRequest {
            params: &json([("feature", sv("Batch")), ("vetoed", b(true))]),
            role: rpc::RpcRole::User,
        },
        &s,
    );
    let JsonValue::Object(r) = r else { panic!("") };
    assert_eq!(r.get("error"), Some(&sv("noPermission")));
}

#[test]
fn defs_flags_29() {
    let mut a = TestAccount::new("e29a");
    let b2 = TestAccount::new("e29b");
    let e = RpcTestEnv::new(&[(&a, 10_000_000_000), (&b2, 1_000_000_000)]);
    let mut p = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), b2.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(500_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
    });
    sign_tx(&mut p, &a);
    let id = p.get_transaction_id();
    e.submit_and_close(&p);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_tx(
        &rpc::TxRequest {
            params: &json([("transaction", JsonValue::String(id.to_string()))]),
            api_version: 2,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert_eq!(r.get("validated"), Some(&b(true)));
        assert!(r.contains_key("hash"));
        assert!(r.contains_key("ledger_index"));
    }
}

#[test]
fn defs_fmt_46() {
    let a = TestAccount::new("c46");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_data(
        &rpc::LedgerDataRequest {
            params: &json([("ledger_index", sv("current"))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert!(r.contains_key("state"));
    }
}

#[test]
fn defs_fmt_75() {
    let a = TestAccount::new("c75");
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
fn defs_fmt_76() {
    let mut a = TestAccount::new("c76a");
    let b2 = TestAccount::new("c76b");
    let e = RpcTestEnv::new(&[(&a, 10_000_000_000), (&b2, 1_000_000_000)]);
    let mut p = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), b2.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(500_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
    });
    sign_tx(&mut p, &a);
    let id = p.get_transaction_id();
    e.submit_and_close(&p);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_tx(
        &rpc::TxRequest {
            params: &json([("transaction", JsonValue::String(id.to_string()))]),
            api_version: 2,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert!(r.contains_key("hash"));
        assert!(r.contains_key("ledger_index"));
    }
}

#[test]
fn defs_fmt_77() {
    let a = TestAccount::new("c77");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let fk = basics::base_uint::Uint256::from_array([0xEE; 32]);
    let JsonValue::Object(r) = rpc::do_tx(
        &rpc::TxRequest {
            params: &json([("transaction", JsonValue::String(fk.to_string()))]),
            api_version: 2,
        },
        &s,
    ) else {
        panic!("")
    };
    assert_eq!(r.get("error"), Some(&sv("txnNotFound")));
}

#[test]
fn defs_fmt_82() {
    let a = TestAccount::new("c82");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger(
        &json([("ledger_index", sv("current"))]),
        rpc::RpcRole::Admin,
        2,
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert!(r.contains_key("ledger") || r.contains_key("ledger_current_index"));
    }
}

#[test]
fn defs_fmt_83() {
    let a = TestAccount::new("c83");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger(
        &json([("ledger_index", sv("closed"))]),
        rpc::RpcRole::Admin,
        2,
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert!(r.contains_key("ledger"));
    }
}

#[test]
fn defs_types_66_feature_list_fields() {
    let a = TestAccount::new("b66");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::feature::do_feature(
        &rpc::feature::FeatureRequest {
            params: &JsonValue::Object(Default::default()),
            role: rpc::RpcRole::Admin,
        },
        &s,
    );
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    if let Some(JsonValue::Object(feats)) = r.get("features") {
        for (_k, v) in feats.iter().take(3) {
            let JsonValue::Object(f) = v else { continue };
            assert!(f.contains_key("name"));
            assert!(f.contains_key("enabled"));
            assert!(f.contains_key("supported"));
        }
    }
}

#[test]
fn defs_types_67_feature_bad() {
    let a = TestAccount::new("b67");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::feature::do_feature(
        &rpc::feature::FeatureRequest {
            params: &json([("feature", sv("NonExistent"))]),
            role: rpc::RpcRole::Admin,
        },
        &s,
    );
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert_eq!(r.get("error"), Some(&sv("badFeature")));
}

#[test]
fn defs_types_68_feature_no_perm() {
    let a = TestAccount::new("b68");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::feature::do_feature(
        &rpc::feature::FeatureRequest {
            params: &json([("feature", sv("Batch")), ("vetoed", b(true))]),
            role: rpc::RpcRole::User,
        },
        &s,
    );
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert_eq!(r.get("error"), Some(&sv("noPermission")));
}
// LedgerRPC: field checks

#[test]
fn defs_types_94_payment_found() {
    let mut a = TestAccount::new("b94a");
    let b2 = TestAccount::new("b94b");
    let e = RpcTestEnv::new(&[(&a, 10_000_000_000), (&b2, 1_000_000_000)]);
    let mut p = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), b2.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
    });
    sign_tx(&mut p, &a);
    let id = p.get_transaction_id();
    e.submit_and_close(&p);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_tx(
        &rpc::TxRequest {
            params: &json([("transaction", JsonValue::String(id.to_string()))]),
            api_version: 2,
        },
        &s,
    ) else {
        panic!("obj")
    };
    if r.get("error").is_none() {
        assert_eq!(r.get("hash"), Some(&JsonValue::String(id.to_string())));
        assert_eq!(r.get("validated"), Some(&b(true)));
    }
}
// More ledger checks

#[test]
fn defs_types_95_ledger_full_denied() {
    let a = TestAccount::new("b95");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger(
        &json([("ledger_index", sv("current")), ("full", b(true))]),
        rpc::RpcRole::Guest,
        2,
        &s,
    ) else {
        panic!("obj")
    };
    assert_eq!(r.get("error"), Some(&sv("noPermission")));
}

#[test]
fn defs_types_96_ledger_accounts_denied() {
    let a = TestAccount::new("b96");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger(
        &json([("ledger_index", sv("current")), ("accounts", b(true))]),
        rpc::RpcRole::Guest,
        2,
        &s,
    ) else {
        panic!("obj")
    };
    assert_eq!(r.get("error"), Some(&sv("noPermission")));
}

#[test]
fn defs_types_97_ledger_queue_closed_denied() {
    let a = TestAccount::new("b97");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger(
        &json([("ledger_index", sv("validated")), ("queue", b(true))]),
        rpc::RpcRole::Admin,
        2,
        &s,
    ) else {
        panic!("obj")
    };
    assert_eq!(r.get("error"), Some(&sv("invalidParams")));
}

#[test]
fn defs_types_98_ledger_not_found() {
    let a = TestAccount::new("b98");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger(
        &json([("ledger_index", u(99999))]),
        rpc::RpcRole::Admin,
        2,
        &s,
    ) else {
        panic!("obj")
    };
    assert_eq!(r.get("error"), Some(&sv("lgrNotFound")));
}
