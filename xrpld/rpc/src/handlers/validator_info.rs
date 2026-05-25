//! Narrow `validator_info` RPC port.
//!
//! This keeps the the reference implementation handler shape:
//!
//! - reject non-validator callers with `invalidParams` / `not a validator`,
//! - resolve the configured validation public key,
//! - map it to the master key through an explicit source seam,
//! - short-circuit when the validation key is already the master key, and
//! - otherwise include manifest details from the same manifest-cache seam used
//!   by the the reference implementation handler.

use std::collections::BTreeMap;

use basics::base64::base64_encode;
use protocol::{JsonValue, NodePublicKey, encode_node_public_base58};

use crate::status::Status;

pub trait ValidatorInfoSource {
    fn get_validation_public_key(&self) -> Option<NodePublicKey>;
    fn get_master_key(&self, validation_public_key: NodePublicKey) -> NodePublicKey;
    fn get_manifest_blob(&self, master_key: NodePublicKey) -> Option<Vec<u8>>;
    fn get_manifest_sequence(&self, master_key: NodePublicKey) -> Option<u32>;
    fn get_manifest_domain(&self, master_key: NodePublicKey) -> Option<String>;
}

pub fn not_validator_error() -> JsonValue {
    let mut result = JsonValue::Object(BTreeMap::new());
    Status::make_param_error("not a validator").inject(&mut result);
    result
}

pub fn do_validator_info<S: ValidatorInfoSource>(source: &S) -> JsonValue {
    let Some(validation_public_key) = source.get_validation_public_key() else {
        return not_validator_error();
    };

    let master_key = source.get_master_key(validation_public_key);
    let mut result = BTreeMap::from([(
        "master_key".to_owned(),
        JsonValue::String(encode_node_public_base58(master_key)),
    )]);

    if master_key == validation_public_key {
        return JsonValue::Object(result);
    }

    result.insert(
        "ephemeral_key".to_owned(),
        JsonValue::String(encode_node_public_base58(validation_public_key)),
    );

    if let Some(manifest) = source.get_manifest_blob(master_key) {
        result.insert(
            "manifest".to_owned(),
            JsonValue::String(base64_encode(&manifest)),
        );
    }
    if let Some(sequence) = source.get_manifest_sequence(master_key) {
        result.insert("seq".to_owned(), JsonValue::Unsigned(u64::from(sequence)));
    }
    if let Some(domain) = source.get_manifest_domain(master_key) {
        result.insert("domain".to_owned(), JsonValue::String(domain));
    }

    JsonValue::Object(result)
}
