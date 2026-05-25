//! Narrow `unl_list` RPC port.
//!
//! This keeps the the reference implementation handler shape and uses a single callback seam
//! for the validator list source instead of inventing a broader runtime owner.

use std::collections::BTreeMap;

use protocol::{JsonValue, NodePublicKey, encode_node_public_base58};

pub trait UnlListSource {
    fn for_each_listed(&self, visitor: &mut dyn FnMut(NodePublicKey, bool));
}

pub fn do_unl_list<S: UnlListSource>(source: &S) -> JsonValue {
    let mut result = BTreeMap::from([("unl".to_owned(), JsonValue::Array(Vec::new()))]);

    let JsonValue::Array(unl) = result.get_mut("unl").expect("unl array must exist") else {
        unreachable!("unl must be an array");
    };

    source.for_each_listed(&mut |public_key, trusted| {
        let node = JsonValue::Object(BTreeMap::from([
            (
                "pubkey_validator".to_owned(),
                JsonValue::String(encode_node_public_base58(public_key)),
            ),
            ("trusted".to_owned(), JsonValue::Bool(trusted)),
        ]));
        unl.push(node);
    });

    JsonValue::Object(result)
}
