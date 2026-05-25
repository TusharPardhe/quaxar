//! Tests for the unl list RPC handler.

use std::cell::RefCell;

use protocol::{JsonValue, NodePublicKey, encode_node_public_base58, parse_base58_node_public};
use rpc::{UnlListSource, do_unl_list};

#[derive(Debug, Default)]
struct FakeUnlListSource {
    entries: Vec<(NodePublicKey, bool)>,
}

impl UnlListSource for FakeUnlListSource {
    fn for_each_listed(&self, visitor: &mut dyn FnMut(NodePublicKey, bool)) {
        for &(public_key, trusted) in &self.entries {
            visitor(public_key, trusted);
        }
    }
}

fn public_key(text: &str) -> NodePublicKey {
    parse_base58_node_public(text).expect("node public key should parse")
}

#[test]
fn unl_list_returns_empty_array() {
    let source = FakeUnlListSource::default();

    let result = do_unl_list(&source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    let JsonValue::Array(unl) = result.get("unl").expect("unl array") else {
        panic!("unl must be an array");
    };
    assert!(unl.is_empty());
}

#[test]
fn unl_list_emits_validator_keys_and_trust_flags_in_callback_order() {
    let source = FakeUnlListSource {
        entries: vec![
            (
                public_key("n949f75evCHwgyP4fPVgaHqNHxUVN15PsJEZ3B3HnXPcPjcZAoy7"),
                true,
            ),
            (
                public_key("nHBt9fsb4849WmZiCds4r5TXyBeQjqnH5kzPtqgMAQMgi39YZRPa"),
                false,
            ),
        ],
    };

    let result = do_unl_list(&source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    let JsonValue::Array(unl) = result.get("unl").expect("unl array") else {
        panic!("unl must be an array");
    };
    assert_eq!(unl.len(), 2);

    let JsonValue::Object(first) = &unl[0] else {
        panic!("first entry must be an object");
    };
    assert_eq!(
        first.get("pubkey_validator"),
        Some(&JsonValue::String(encode_node_public_base58(public_key(
            "n949f75evCHwgyP4fPVgaHqNHxUVN15PsJEZ3B3HnXPcPjcZAoy7"
        ))))
    );
    assert_eq!(first.get("trusted"), Some(&JsonValue::Bool(true)));

    let JsonValue::Object(second) = &unl[1] else {
        panic!("second entry must be an object");
    };
    assert_eq!(
        second.get("pubkey_validator"),
        Some(&JsonValue::String(encode_node_public_base58(public_key(
            "nHBt9fsb4849WmZiCds4r5TXyBeQjqnH5kzPtqgMAQMgi39YZRPa"
        ))))
    );
    assert_eq!(second.get("trusted"), Some(&JsonValue::Bool(false)));
}

#[test]
fn unl_list_preserves_source_order() {
    let call_order = RefCell::new(Vec::new());
    struct OrderedSource<'a> {
        entries: Vec<(NodePublicKey, bool)>,
        call_order: &'a RefCell<Vec<NodePublicKey>>,
    }

    impl UnlListSource for OrderedSource<'_> {
        fn for_each_listed(&self, visitor: &mut dyn FnMut(NodePublicKey, bool)) {
            for &(public_key, trusted) in &self.entries {
                self.call_order.borrow_mut().push(public_key);
                visitor(public_key, trusted);
            }
        }
    }

    let source = OrderedSource {
        entries: vec![
            (
                public_key("n949f75evCHwgyP4fPVgaHqNHxUVN15PsJEZ3B3HnXPcPjcZAoy7"),
                true,
            ),
            (
                public_key("nHBt9fsb4849WmZiCds4r5TXyBeQjqnH5kzPtqgMAQMgi39YZRPa"),
                false,
            ),
        ],
        call_order: &call_order,
    };

    let result = do_unl_list(&source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Array(unl) = result.get("unl").expect("unl array") else {
        panic!("unl must be an array");
    };
    assert_eq!(unl.len(), 2);
    assert_eq!(call_order.borrow().len(), 2);
}
