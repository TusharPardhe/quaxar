//! Narrow `manifest` RPC handler port.

use std::collections::BTreeMap;

use basics::base64::base64_encode;
use protocol::{JsonValue, NodePublicKey, encode_node_public_base58, parse_base58_node_public};

use crate::commands::rpc_helpers::{inject_error, missing_field_error};

fn json_value_as_string(value: &JsonValue) -> String {
    match value {
        JsonValue::String(text) => text.clone(),
        JsonValue::Signed(value) => value.to_string(),
        JsonValue::Unsigned(value) => value.to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => String::new(),
    }
}

pub trait ManifestSource {
    fn get_master_key(&self, requested: NodePublicKey) -> Option<NodePublicKey>;

    fn get_signing_key(&self, master_key: NodePublicKey) -> Option<NodePublicKey>;

    fn get_manifest_blob(&self, master_key: NodePublicKey) -> Option<Vec<u8>>;

    fn get_manifest_sequence(&self, master_key: NodePublicKey) -> Option<u32>;

    fn get_manifest_domain(&self, master_key: NodePublicKey) -> Option<String>;
}

pub fn do_manifest<S: ManifestSource>(params: &JsonValue, source: &S) -> JsonValue {
    let JsonValue::Object(object) = params else {
        return missing_field_error("public_key");
    };

    let Some(public_key) = object.get("public_key") else {
        return missing_field_error("public_key");
    };
    let requested = json_value_as_string(public_key);

    let mut result =
        BTreeMap::from([("requested".to_owned(), JsonValue::String(requested.clone()))]);

    let Some(requested_key) = parse_base58_node_public(&requested) else {
        let mut json = JsonValue::Object(result);
        inject_error(crate::RpcErrorCode::InvalidParams, &mut json);
        return json;
    };

    let Some(master_key) = source.get_master_key(requested_key) else {
        return JsonValue::Object(result);
    };
    let Some(ephemeral_key) = source.get_signing_key(master_key) else {
        return JsonValue::Object(result);
    };

    if let Some(manifest) = source.get_manifest_blob(master_key) {
        result.insert(
            "manifest".to_owned(),
            JsonValue::String(base64_encode(&manifest)),
        );
    }

    let mut details = BTreeMap::from([
        (
            "master_key".to_owned(),
            JsonValue::String(encode_node_public_base58(master_key)),
        ),
        (
            "ephemeral_key".to_owned(),
            JsonValue::String(encode_node_public_base58(ephemeral_key)),
        ),
    ]);
    if let Some(sequence) = source.get_manifest_sequence(master_key) {
        details.insert("seq".to_owned(), JsonValue::Unsigned(u64::from(sequence)));
    }
    if let Some(domain) = source.get_manifest_domain(master_key) {
        details.insert("domain".to_owned(), JsonValue::String(domain));
    }
    result.insert("details".to_owned(), JsonValue::Object(details));

    JsonValue::Object(result)
}
