//! Tests for the validator info RPC handler.

use std::collections::BTreeMap;

use basics::base64::base64_decode;
use protocol::{JsonValue, NodePublicKey, encode_node_public_base58, parse_base58_node_public};
use rpc::{ValidatorInfoSource, do_validator_info, not_validator_error};

#[derive(Debug, Default)]
struct FakeValidatorInfoSource {
    validation_public_key: Option<NodePublicKey>,
    master_for_validation: BTreeMap<NodePublicKey, NodePublicKey>,
    manifest_for_master: BTreeMap<NodePublicKey, Vec<u8>>,
    sequence_for_master: BTreeMap<NodePublicKey, u32>,
    domain_for_master: BTreeMap<NodePublicKey, String>,
}

impl ValidatorInfoSource for FakeValidatorInfoSource {
    fn get_validation_public_key(&self) -> Option<NodePublicKey> {
        self.validation_public_key
    }

    fn get_master_key(&self, validation_public_key: NodePublicKey) -> NodePublicKey {
        self.master_for_validation
            .get(&validation_public_key)
            .copied()
            .unwrap_or(validation_public_key)
    }

    fn get_manifest_blob(&self, master_key: NodePublicKey) -> Option<Vec<u8>> {
        self.manifest_for_master.get(&master_key).cloned()
    }

    fn get_manifest_sequence(&self, master_key: NodePublicKey) -> Option<u32> {
        self.sequence_for_master.get(&master_key).copied()
    }

    fn get_manifest_domain(&self, master_key: NodePublicKey) -> Option<String> {
        self.domain_for_master.get(&master_key).cloned()
    }
}

fn requested_key() -> NodePublicKey {
    parse_base58_node_public("n949f75evCHwgyP4fPVgaHqNHxUVN15PsJEZ3B3HnXPcPjcZAoy7")
        .expect("requested key should parse")
}

fn master_key() -> NodePublicKey {
    parse_base58_node_public("nHBt9fsb4849WmZiCds4r5TXyBeQjqnH5kzPtqgMAQMgi39YZRPa")
        .expect("master key should parse")
}

#[test]
fn validator_info_returns_not_validator_error() {
    let source = FakeValidatorInfoSource::default();

    let result = do_validator_info(&source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );
    assert_eq!(
        result.get("error_message"),
        Some(&JsonValue::String("not a validator".to_owned()))
    );
}

#[test]
fn validator_info_returns_master_only_when_validation_key_is_already_master() {
    let validation = requested_key();
    let mut source = FakeValidatorInfoSource {
        validation_public_key: Some(validation),
        ..Default::default()
    };
    source.master_for_validation.insert(validation, validation);

    let result = do_validator_info(&source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        result.get("master_key"),
        Some(&JsonValue::String(encode_node_public_base58(validation)))
    );
    assert!(!result.contains_key("ephemeral_key"));
    assert!(!result.contains_key("manifest"));
    assert!(!result.contains_key("seq"));
    assert!(!result.contains_key("domain"));
}

#[test]
fn validator_info_returns_full_manifest_details() {
    const MANIFEST_TOKEN: &str = "JAAAAAFxIe1FtwmimvGtH2iCcMJqC9gVFKilGfw1/vCxHXXLplc2GnMhAkE1agqXxBwDwDbID6OMSYuM0FDAlpAgNk8SKFn7MO2fdkcwRQIhAOngu9sAKqXYouJ+l2V0W+sAOkVB+ZRS6PShlJAfUsXfAiBsVJGesaadOJc/aAZokS1vymGmVrlHPKWX3Yywu6in8HASQKPugBD67kMaRFGvmpATHlGKJdvDFlWPYy5AqDedFv5TJa2w0i21eq3MYywLVJZnFOr7C0kw2AiTzSCjIzditQ8=";

    let validation = requested_key();
    let master = master_key();
    let mut source = FakeValidatorInfoSource {
        validation_public_key: Some(validation),
        ..Default::default()
    };
    source.master_for_validation.insert(validation, master);
    source
        .manifest_for_master
        .insert(master, base64_decode(MANIFEST_TOKEN));
    source.sequence_for_master.insert(master, 7);
    source
        .domain_for_master
        .insert(master, "example.com".to_owned());

    let result = do_validator_info(&source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        result.get("master_key"),
        Some(&JsonValue::String(encode_node_public_base58(master)))
    );
    assert_eq!(
        result.get("ephemeral_key"),
        Some(&JsonValue::String(encode_node_public_base58(validation)))
    );
    assert_eq!(
        result.get("manifest"),
        Some(&JsonValue::String(MANIFEST_TOKEN.to_owned()))
    );
    assert_eq!(result.get("seq"), Some(&JsonValue::Unsigned(7)));
    assert_eq!(
        result.get("domain"),
        Some(&JsonValue::String("example.com".to_owned()))
    );
    assert_ne!(validation, master);
}

#[test]
fn validator_info_not_validator_error_helper_shape() {
    let JsonValue::Object(result) = not_validator_error() else {
        panic!("error must be an object");
    };

    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );
}
