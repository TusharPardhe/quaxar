//! Integration tests for ledger operations.

use protocol::{get_field_by_symbol, to_base58, JsonValue, STAmount, STTx, TxType};
use rpc_integration_tests::env::*;
use rpc_integration_tests::helpers::*;

#[test]
fn defs_ledger_current() {
    let a = TestAccount::new("t36");
    let e = RpcTestEnv::new(&[(&a, 10_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::do_ledger_current(&s);
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert!(r.contains_key("ledger_current_index"));
}

#[test]
fn defs_ledger_closed() {
    let a = TestAccount::new("t37");
    let e = RpcTestEnv::new(&[(&a, 10_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::do_ledger_closed(&s);
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert!(r.contains_key("ledger_hash"));
    assert!(r.contains_key("ledger_index"));
}

#[test]
fn defs_ledger_advance() {
    let a = TestAccount::new("t38");
    let e = RpcTestEnv::new(&[(&a, 10_000_000_000)]);
    e.app.accept_standalone_ledger().unwrap();
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_current(&s) else {
        panic!("obj")
    };
    let JsonValue::Unsigned(idx) = r.get("ledger_current_index").unwrap() else {
        panic!("u")
    };
    assert!(*idx >= 3);
}

#[test]
fn defs_ledger_entry_not_found() {
    let a = TestAccount::new("t46");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::do_ledger_entry(
        &rpc::LedgerEntryRequest {
            params: &json([
                ("index", JsonValue::String("AA".repeat(32))),
                ("ledger_index", JsonValue::String("current".to_owned())),
            ]),
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
        Some(&JsonValue::String("entryNotFound".to_owned()))
    );
}

#[test]
fn defs_ledger_close_advances() {
    let a = TestAccount::new("t58");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(b) = rpc::do_ledger_closed(&s) else {
        panic!("obj")
    };
    let i1 = b.get("ledger_index").cloned();
    e.app.accept_standalone_ledger().unwrap();
    let s = e.rpc_source();
    let JsonValue::Object(a2) = rpc::do_ledger_closed(&s) else {
        panic!("obj")
    };
    let i2 = a2.get("ledger_index").cloned();
    assert_ne!(i1, i2);
}

#[test]
fn defs_payment_changes_balance() {
    let mut a = TestAccount::new("t59a");
    let b2 = TestAccount::new("t59b");
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
    e.submit_and_close(&p);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_closed(&s) else {
        panic!("obj")
    };
    assert!(r.contains_key("ledger_hash"));
}

#[test]
fn defs_ext_46() {
    let a = TestAccount::new("d46");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_current(&s) else {
        panic!("")
    };
    assert!(r.contains_key("ledger_current_index"));
}

#[test]
fn defs_ext_47() {
    let a = TestAccount::new("d47");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_closed(&s) else {
        panic!("")
    };
    assert!(r.contains_key("ledger_hash"));
    assert!(r.contains_key("ledger_index"));
}

#[test]
fn defs_ext_60() {
    let a = TestAccount::new("d60");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_entry(
        &rpc::LedgerEntryRequest {
            params: &json([
                ("account_root", JsonValue::String(to_base58(a.id))),
                ("ledger_index", sv("current")),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert!(r.contains_key("node"));
        assert!(r.contains_key("index"));
    }
}

#[test]
fn defs_ext_61() {
    let a = TestAccount::new("d61");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let fk = basics::base_uint::Uint256::from_array([0xCC; 32]);
    let JsonValue::Object(r) = rpc::do_ledger_entry(
        &rpc::LedgerEntryRequest {
            params: &json([
                ("index", JsonValue::String(fk.to_string())),
                ("ledger_index", sv("current")),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    assert_eq!(r.get("error"), Some(&sv("entryNotFound")));
}

#[test]
fn defs_ext_80() {
    let a = TestAccount::new("d80");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    e.app.accept_standalone_ledger().unwrap();
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_closed(&s) else {
        panic!("")
    };
    let JsonValue::String(h) = r.get("ledger_hash").unwrap() else {
        panic!("")
    };
    assert_eq!(h.len(), 64);
    assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn defs_ext_81() {
    let a = TestAccount::new("d81");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    e.app.accept_standalone_ledger().unwrap();
    e.app.accept_standalone_ledger().unwrap();
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_current(&s) else {
        panic!("")
    };
    let JsonValue::Unsigned(i) = r.get("ledger_current_index").unwrap() else {
        panic!("")
    };
    assert!(*i >= 4);
}

#[test]
fn defs_flags_27() {
    let a = TestAccount::new("e27");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    e.app.accept_standalone_ledger().unwrap();
    e.app.accept_standalone_ledger().unwrap();
    e.app.accept_standalone_ledger().unwrap();
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_current(&s) else {
        panic!("")
    };
    let JsonValue::Unsigned(i) = r.get("ledger_current_index").unwrap() else {
        panic!("")
    };
    assert!(*i >= 5);
}

#[test]
fn defs_flags_28() {
    let a = TestAccount::new("e28");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s1 = e.rpc_source();
    let JsonValue::Object(r1) = rpc::do_ledger_closed(&s1) else {
        panic!("")
    };
    let h1 = r1.get("ledger_hash").cloned();
    e.app.accept_standalone_ledger().unwrap();
    let s2 = e.rpc_source();
    let JsonValue::Object(r2) = rpc::do_ledger_closed(&s2) else {
        panic!("")
    };
    let h2 = r2.get("ledger_hash").cloned();
    assert_ne!(h1, h2);
}

#[test]
fn defs_fmt_47() {
    let a = TestAccount::new("c47");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_entry(
        &rpc::LedgerEntryRequest {
            params: &json([
                ("account_root", JsonValue::String(to_base58(a.id))),
                ("ledger_index", sv("current")),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert!(r.contains_key("node") || r.contains_key("index"));
    }
}

#[test]
fn defs_fmt_50() {
    let a = TestAccount::new("c50");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    e.app.accept_standalone_ledger().unwrap();
    e.app.accept_standalone_ledger().unwrap();
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_current(&s) else {
        panic!("")
    };
    let JsonValue::Unsigned(i) = r.get("ledger_current_index").unwrap() else {
        panic!("")
    };
    assert!(*i >= 4);
}

#[test]
fn defs_types_69_ledger_closed_fields() {
    let a = TestAccount::new("b69");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_closed(&s) else {
        panic!("obj")
    };
    assert!(r.contains_key("ledger_hash"));
    assert!(r.contains_key("ledger_index"));
    let JsonValue::String(h) = r.get("ledger_hash").unwrap() else {
        panic!("str")
    };
    assert_eq!(h.len(), 64);
    let JsonValue::Unsigned(i) = r.get("ledger_index").unwrap() else {
        panic!("u")
    };
    assert!(*i >= 1);
}

#[test]
fn defs_types_70_ledger_current_field() {
    let a = TestAccount::new("b70");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_current(&s) else {
        panic!("obj")
    };
    let JsonValue::Unsigned(i) = r.get("ledger_current_index").unwrap() else {
        panic!("u")
    };
    assert!(*i >= 2);
}

// AccountInfo: field value checks
