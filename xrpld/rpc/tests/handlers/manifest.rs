//! Tests for the manifest RPC handler.

use std::collections::BTreeMap;

use basics::base64::base64_decode;
use protocol::{JsonValue, NodePublicKey, encode_node_public_base58, parse_base58_node_public};
use rpc::{ManifestSource, do_manifest};

#[derive(Debug, Default)]
struct FakeManifestSource {
    master_for_requested: BTreeMap<NodePublicKey, NodePublicKey>,
    signing_for_master: BTreeMap<NodePublicKey, NodePublicKey>,
    manifest_for_master: BTreeMap<NodePublicKey, Vec<u8>>,
    sequence_for_master: BTreeMap<NodePublicKey, u32>,
    domain_for_master: BTreeMap<NodePublicKey, String>,
}

impl ManifestSource for FakeManifestSource {
    fn get_master_key(&self, requested: NodePublicKey) -> Option<NodePublicKey> {
        self.master_for_requested.get(&requested).copied()
    }

    fn get_signing_key(&self, master_key: NodePublicKey) -> Option<NodePublicKey> {
        self.signing_for_master.get(&master_key).copied()
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

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn requested_key() -> NodePublicKey {
    parse_base58_node_public("n949f75evCHwgyP4fPVgaHqNHxUVN15PsJEZ3B3HnXPcPjcZAoy7")
        .expect("requested key should parse")
}

fn master_key() -> NodePublicKey {
    parse_base58_node_public("nHBt9fsb4849WmZiCds4r5TXyBeQjqnH5kzPtqgMAQMgi39YZRPa")
        .expect("master key should parse")
}

fn signing_key() -> NodePublicKey {
    parse_base58_node_public("n9KsDYGKhABVc4wK5u3MnVhgPinyJimyKGpr9VJYuBaY8EnJXR2x")
        .expect("signing key should parse")
}

#[test]
fn manifest_reports_missing_and_invalid_public_keys() {
    let source = FakeManifestSource::default();

    let missing = do_manifest(&JsonValue::Object(Default::default()), &source);
    let JsonValue::Object(missing) = missing else {
        panic!("missing result must be an object");
    };
    assert_eq!(
        missing.get("error_message"),
        Some(&JsonValue::String("Missing field 'public_key'.".to_owned()))
    );

    let invalid = do_manifest(
        &object([("public_key", JsonValue::String("abcdef12345".to_owned()))]),
        &source,
    );
    let JsonValue::Object(invalid) = invalid else {
        panic!("invalid result must be an object");
    };
    assert_eq!(
        invalid.get("requested"),
        Some(&JsonValue::String("abcdef12345".to_owned()))
    );
    assert_eq!(
        invalid.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );
}

#[test]
fn manifest_echoes_requested_and_returns_lookup_details() {
    const MANIFEST_TOKEN: &str = "JAAAAAFxIe1FtwmimvGtH2iCcMJqC9gVFKilGfw1/vCxHXXLplc2GnMhAkE1agqXxBwDwDbID6OMSYuM0FDAlpAgNk8SKFn7MO2fdkcwRQIhAOngu9sAKqXYouJ+l2V0W+sAOkVB+ZRS6PShlJAfUsXfAiBsVJGesaadOJc/aAZokS1vymGmVrlHPKWX3Yywu6in8HASQKPugBD67kMaRFGvmpATHlGKJdvDFlWPYy5AqDedFv5TJa2w0i21eq3MYywLVJZnFOr7C0kw2AiTzSCjIzditQ8=";

    let requested = requested_key();
    let master = master_key();
    let signing = signing_key();

    let mut source = FakeManifestSource::default();
    source.master_for_requested.insert(requested, master);
    source.signing_for_master.insert(master, signing);
    source
        .manifest_for_master
        .insert(master, base64_decode(MANIFEST_TOKEN));
    source.sequence_for_master.insert(master, 1);
    source
        .domain_for_master
        .insert(master, "example.com".to_owned());

    let result = do_manifest(
        &object([(
            "public_key",
            JsonValue::String(encode_node_public_base58(requested)),
        )]),
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        result.get("requested"),
        Some(&JsonValue::String(
            "n949f75evCHwgyP4fPVgaHqNHxUVN15PsJEZ3B3HnXPcPjcZAoy7".to_owned()
        ))
    );
    assert_eq!(
        result.get("manifest"),
        Some(&JsonValue::String(MANIFEST_TOKEN.to_owned()))
    );

    let JsonValue::Object(details) = result.get("details").expect("details must exist") else {
        panic!("details must be an object");
    };
    assert_eq!(
        details.get("master_key"),
        Some(&JsonValue::String(
            "nHBt9fsb4849WmZiCds4r5TXyBeQjqnH5kzPtqgMAQMgi39YZRPa".to_owned()
        ))
    );
    assert_eq!(
        details.get("ephemeral_key"),
        Some(&JsonValue::String(
            "n9KsDYGKhABVc4wK5u3MnVhgPinyJimyKGpr9VJYuBaY8EnJXR2x".to_owned()
        ))
    );
    assert_eq!(details.get("seq"), Some(&JsonValue::Unsigned(1)));
    assert_eq!(
        details.get("domain"),
        Some(&JsonValue::String("example.com".to_owned()))
    );
}

#[test]
fn manifest_returns_requested_only_when_lookup_fails() {
    let requested = requested_key();
    let source = FakeManifestSource::default();

    let result = do_manifest(
        &object([(
            "public_key",
            JsonValue::String(encode_node_public_base58(requested)),
        )]),
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        result.get("requested"),
        Some(&JsonValue::String(
            "n949f75evCHwgyP4fPVgaHqNHxUVN15PsJEZ3B3HnXPcPjcZAoy7".to_owned()
        ))
    );
    assert!(!result.contains_key("details"));
    assert!(!result.contains_key("manifest"));
}

#[test]
fn manifest_non_string_public_key_rejected() {
    let source = FakeManifestSource::default();

    let result = do_manifest(&object([("public_key", JsonValue::Unsigned(42))]), &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert!(result.contains_key("error") || result.contains_key("error_message"));
}

#[test]
fn manifest_with_master_key_directly() {
    let master = master_key();
    let signing = signing_key();

    let mut source = FakeManifestSource::default();
    source.master_for_requested.insert(master, master);
    source.signing_for_master.insert(master, signing);
    source.sequence_for_master.insert(master, 5);

    let result = do_manifest(
        &object([(
            "public_key",
            JsonValue::String(encode_node_public_base58(master)),
        )]),
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("requested"),
        Some(&JsonValue::String(encode_node_public_base58(master)))
    );
    assert!(result.contains_key("details"));
    let JsonValue::Object(details) = result.get("details").unwrap() else {
        panic!("details must be an object");
    };
    assert_eq!(
        details.get("master_key"),
        Some(&JsonValue::String(encode_node_public_base58(master)))
    );
    assert_eq!(details.get("seq"), Some(&JsonValue::Unsigned(5)));
}
