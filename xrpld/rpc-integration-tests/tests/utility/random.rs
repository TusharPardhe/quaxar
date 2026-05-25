//! Integration tests for random operations.

use protocol::JsonValue;

#[test]
fn random_returns_unique_values() {
    let a = rpc::do_random();
    let b = rpc::do_random();
    assert_ne!(a, b);
}

#[test]
fn random_returns_64_char_hex() {
    let JsonValue::Object(r) = rpc::do_random() else {
        panic!("obj")
    };
    let JsonValue::String(v) = r.get("random").unwrap() else {
        panic!("str")
    };
    assert_eq!(v.len(), 64);
}

#[test]
fn random_returns_object() {
    let r = rpc::do_random();
    assert!(matches!(r, JsonValue::Object(_)));
}

#[test]
fn defs_ext_1() {
    assert!(matches!(rpc::do_random(), JsonValue::Object(_)));
}

#[test]
fn defs_ext_2() {
    let JsonValue::Object(r) = rpc::do_random() else {
        panic!("")
    };
    assert_eq!(r.len(), 1);
}

#[test]
fn defs_ext_3() {
    let JsonValue::Object(r) = rpc::do_random() else {
        panic!("")
    };
    let JsonValue::String(v) = r.get("random").unwrap() else {
        panic!("")
    };
    assert_eq!(v.len(), 64);
}

#[test]
fn defs_ext_4() {
    let JsonValue::Object(r) = rpc::do_random() else {
        panic!("")
    };
    let JsonValue::String(v) = r.get("random").unwrap() else {
        panic!("")
    };
    assert!(v.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn defs_ext_5() {
    let JsonValue::Object(a) = rpc::do_random() else {
        panic!("")
    };
    let JsonValue::Object(b2) = rpc::do_random() else {
        panic!("")
    };
    assert_ne!(a.get("random"), b2.get("random"));
}

#[test]
fn defs_ext_6() {
    for _ in 0..10 {
        let JsonValue::Object(r) = rpc::do_random() else {
            panic!("")
        };
        let JsonValue::String(v) = r.get("random").unwrap() else {
            panic!("")
        };
        assert_ne!(
            v,
            "0000000000000000000000000000000000000000000000000000000000000000"
        );
    }
}

#[test]
fn defs_fmt_7() {
    let r = rpc::do_random();
    assert!(matches!(r, JsonValue::Object(_)));
}

#[test]
fn defs_fmt_8() {
    let JsonValue::Object(r) = rpc::do_random() else {
        panic!("")
    };
    assert!(r.contains_key("random"));
}

#[test]
fn defs_fmt_9() {
    let JsonValue::Object(r) = rpc::do_random() else {
        panic!("")
    };
    assert!(!r.contains_key("error"));
}

#[test]
fn defs_fmt_10() {
    let JsonValue::Object(r1) = rpc::do_random() else {
        panic!("")
    };
    let JsonValue::Object(r2) = rpc::do_random() else {
        panic!("")
    };
    assert_ne!(r1.get("random"), r2.get("random"));
}

#[test]
fn defs_types_91_random_hex_valid() {
    for _ in 0..5 {
        let JsonValue::Object(r) = rpc::do_random() else {
            panic!("obj")
        };
        let JsonValue::String(v) = r.get("random").unwrap() else {
            panic!("str")
        };
        assert!(v.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

#[test]
fn defs_types_92_random_not_zero() {
    let JsonValue::Object(r) = rpc::do_random() else {
        panic!("obj")
    };
    let JsonValue::String(v) = r.get("random").unwrap() else {
        panic!("str")
    };
    assert_ne!(v, &"0".repeat(64));
}
// Integration: trust line creates objects
