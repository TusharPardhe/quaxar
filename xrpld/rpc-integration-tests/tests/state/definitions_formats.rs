//! Tests for definitions formats.

use protocol::{
    currency_from_string, get_field_by_symbol, lsfDefaultRipple, to_base58, Issue, JsonValue,
    STAmount, STTx, TxType,
};
use rpc_integration_tests::env::*;
use rpc_integration_tests::helpers::*;
use server::{StreamKind, SubscriptionManager};
// ServerDefinitions format field name checks
#[test]
fn definitions_format_1() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    let JsonValue::Array(p) = f.get("Payment").unwrap() else {
        panic!("")
    };
    let names: Vec<&str> = p
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
    assert!(names.contains(&"Destination"));
    assert!(names.contains(&"Amount"));
}
#[test]
fn definitions_format_2() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    let JsonValue::Array(p) = f.get("OfferCreate").unwrap() else {
        panic!("")
    };
    let names: Vec<&str> = p
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
    assert!(names.contains(&"TakerPays"));
    assert!(names.contains(&"TakerGets"));
}
#[test]
fn definitions_format_3() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    let JsonValue::Array(p) = f.get("TrustSet").unwrap() else {
        panic!("")
    };
    let names: Vec<&str> = p
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
    assert!(names.contains(&"LimitAmount"));
}
#[test]
fn definitions_format_4() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    let JsonValue::Array(p) = f.get("EscrowCreate").unwrap() else {
        panic!("")
    };
    let names: Vec<&str> = p
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
    assert!(names.contains(&"Destination"));
    assert!(names.contains(&"Amount"));
}
#[test]
fn definitions_format_5() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    let JsonValue::Array(p) = f.get("AccountRoot").unwrap() else {
        panic!("")
    };
    let names: Vec<&str> = p
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
    assert!(names.contains(&"Account"));
    assert!(names.contains(&"Sequence"));
    assert!(names.contains(&"Balance"));
}
#[test]
fn definitions_format_6() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    let JsonValue::Array(p) = f.get("Offer").unwrap() else {
        panic!("")
    };
    let names: Vec<&str> = p
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
    assert!(names.contains(&"Account"));
    assert!(names.contains(&"TakerPays"));
    assert!(names.contains(&"TakerGets"));
}
#[test]
fn definitions_format_7() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    let JsonValue::Array(p) = f.get("RippleState").unwrap() else {
        panic!("")
    };
    assert!(!p.is_empty());
}
#[test]
fn definitions_format_8() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    let JsonValue::Array(p) = f.get("Escrow").unwrap() else {
        panic!("")
    };
    assert!(!p.is_empty());
}
#[test]
fn definitions_format_9() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    let JsonValue::Array(p) = f.get("PayChannel").unwrap() else {
        panic!("")
    };
    assert!(!p.is_empty());
}
#[test]
fn definitions_format_10() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    let JsonValue::Array(p) = f.get("Check").unwrap() else {
        panic!("")
    };
    assert!(!p.is_empty());
}
// Format field optionality checks
#[test]
fn definitions_format_11() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    for (_name, fields) in f.iter().take(5) {
        let JsonValue::Array(fields) = fields else {
            continue;
        };
        for field in fields {
            let JsonValue::Object(fo) = field else {
                continue;
            };
            assert!(fo.contains_key("name"));
            assert!(fo.contains_key("optionality"));
        }
    }
}
#[test]
fn definitions_format_12() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    for (_name, fields) in f.iter().take(5) {
        let JsonValue::Array(fields) = fields else {
            continue;
        };
        for field in fields {
            let JsonValue::Object(fo) = field else {
                continue;
            };
            assert!(fo.contains_key("name"));
            assert!(fo.contains_key("optionality"));
        }
    }
}
// FIELDS array structure
#[test]
fn definitions_format_13() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Array(f) = r.get("FIELDS").unwrap() else {
        panic!("")
    };
    for entry in f.iter().take(10) {
        let JsonValue::Array(e) = entry else {
            panic!("")
        };
        assert_eq!(e.len(), 2);
        assert!(matches!(&e[0], JsonValue::String(_)));
        assert!(matches!(&e[1], JsonValue::Object(_)));
    }
}
#[test]
fn definitions_format_14() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Array(f) = r.get("FIELDS").unwrap() else {
        panic!("")
    };
    for entry in f.iter().take(10) {
        let JsonValue::Array(e) = entry else {
            panic!("")
        };
        let JsonValue::Object(props) = &e[1] else {
            panic!("")
        };
        assert!(props.contains_key("nth"));
        assert!(props.contains_key("isVLEncoded"));
        assert!(props.contains_key("isSerialized"));
        assert!(props.contains_key("isSigningField"));
        assert!(props.contains_key("type"));
    }
}
// FIELDS unique names
#[test]
fn definitions_format_15() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Array(f) = r.get("FIELDS").unwrap() else {
        panic!("")
    };
    let mut names = std::collections::HashSet::new();
    for entry in f {
        let JsonValue::Array(e) = entry else { continue };
        let JsonValue::String(name) = &e[0] else {
            continue;
        };
        assert!(names.insert(name.clone()), "duplicate: {name}");
    }
}
// FIELDS special entries
#[test]
fn definitions_format_16() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Array(f) = r.get("FIELDS").unwrap() else {
        panic!("")
    };
    let names: Vec<&str> = f
        .iter()
        .filter_map(|e| {
            if let JsonValue::Array(a) = e {
                if let JsonValue::String(n) = &a[0] {
                    Some(n.as_str())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();
    assert!(!names.contains(&"Generic"));
    assert!(names.contains(&"hash"));
    assert!(names.contains(&"index"));
}
// TYPES specific values
#[test]
fn definitions_format_17() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("Hash256"), Some(&si(5)));
    assert_eq!(t.get("Amount"), Some(&si(6)));
    assert_eq!(t.get("AccountID"), Some(&si(8)));
}
#[test]
fn definitions_format_18() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("UInt16"), Some(&si(1)));
    assert_eq!(t.get("UInt32"), Some(&si(2)));
    assert_eq!(t.get("UInt64"), Some(&si(3)));
}
#[test]
fn definitions_format_19() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("Hash128"), Some(&si(4)));
    assert_eq!(t.get("Hash160"), Some(&si(17)));
}
// LEDGER_ENTRY_TYPES specific values
#[test]
fn definitions_format_20() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("AccountRoot"), Some(&si(97)));
    assert_eq!(t.get("Offer"), Some(&si(111)));
    assert_eq!(t.get("RippleState"), Some(&si(114)));
    assert_eq!(t.get("DirectoryNode"), Some(&si(100)));
}
// Integration: multiple trust lines visible
#[test]
fn definitions_format_21() {
    let mut a = TestAccount::new("f21a");
    let g = TestAccount::new("f21g");
    let e = RpcTestEnv::with_flags(
        &[(&a, 10_000_000_000), (&g, 10_000_000_000)],
        &[(&g, lsfDefaultRipple)],
    );
    for cur in ["USD", "EUR", "GBP"] {
        let mut t = STTx::new(TxType::TRUST_SET, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
            tx.set_field_amount(
                get_field_by_symbol("sfLimitAmount"),
                STAmount::new_with_asset(
                    get_field_by_symbol("sfLimitAmount"),
                    Issue::new(currency_from_string(cur), g.id),
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
    }
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
        assert!(l.len() >= 3);
    }
}
// Integration: offer visible in account_objects
#[test]
fn definitions_format_22() {
    let mut a = TestAccount::new("f22a");
    let g = TestAccount::new("f22g");
    let e = RpcTestEnv::with_flags(
        &[(&a, 10_000_000_000), (&g, 10_000_000_000)],
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
                10000,
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
    let mut o = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(100_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfTakerGets"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfTakerGets"),
                Issue::new(usd, g.id),
                50,
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
    sign_tx(&mut o, &a);
    e.submit_and_close(&o);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(a.id))),
                ("type", sv("offer")),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Array(objs)) = r.get("account_objects") {
        if !objs.is_empty() {
            let JsonValue::Object(of) = &objs[0] else {
                panic!("")
            };
            assert_eq!(of.get("LedgerEntryType"), Some(&sv("Offer")));
            assert!(of.contains_key("TakerPays"));
            assert!(of.contains_key("TakerGets"));
            assert!(of.contains_key("Sequence"));
        }
    }
}
// More subscribe receiver count checks
#[test]
fn definitions_format_23() {
    let m = SubscriptionManager::new(8);
    assert_eq!(m.publish_json(StreamKind::Ledger, obj()), 0);
    assert_eq!(m.publish_json(StreamKind::Transactions, obj()), 0);
    assert_eq!(m.publish_json(StreamKind::Server, obj()), 0);
}
#[test]
fn definitions_format_24() {
    let m = SubscriptionManager::new(8);
    let _r = m.subscribe(StreamKind::Ledger);
    assert_eq!(m.publish_json(StreamKind::Ledger, obj()), 1);
    assert_eq!(m.publish_json(StreamKind::Transactions, obj()), 0);
}
#[test]
fn definitions_format_25() {
    let m = SubscriptionManager::new(8);
    let _r = m.subscribe(StreamKind::Transactions);
    assert_eq!(m.publish_json(StreamKind::Transactions, obj()), 1);
    assert_eq!(m.publish_json(StreamKind::Ledger, obj()), 0);
}
