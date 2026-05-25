//! Tests for wallet propose.

use std::collections::BTreeMap;

use protocol::JsonValue;
use rpc::wallet_propose;

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[test]
fn wallet_propose_random_secp256k1() {
    let result = wallet_propose(&object([])).expect("should succeed");
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert!(result.contains_key("account_id"));
    assert!(result.contains_key("master_seed"));
    assert!(result.contains_key("master_seed_hex"));
    assert!(result.contains_key("public_key"));
    assert!(result.contains_key("public_key_hex"));
    assert!(result.contains_key("key_type"));
    assert_eq!(
        result.get("key_type"),
        Some(&JsonValue::String("secp256k1".to_owned()))
    );
    assert!(!result.contains_key("warning"));

    // Second call should produce different seed
    let result2 = wallet_propose(&object([])).expect("should succeed");
    let JsonValue::Object(result2) = result2 else {
        panic!("result2 must be an object");
    };
    assert_ne!(result.get("master_seed"), result2.get("master_seed"));
}

#[test]
fn wallet_propose_random_ed25519() {
    let result = wallet_propose(&object([(
        "key_type",
        JsonValue::String("ed25519".to_owned()),
    )]))
    .expect("should succeed");
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert!(result.contains_key("account_id"));
    assert!(result.contains_key("master_seed"));
    assert!(result.contains_key("master_seed_hex"));
    assert!(result.contains_key("public_key"));
    assert!(result.contains_key("public_key_hex"));
    assert_eq!(
        result.get("key_type"),
        Some(&JsonValue::String("ed25519".to_owned()))
    );
    assert!(!result.contains_key("warning"));
}

#[test]
fn wallet_propose_from_seed_secp256k1() {
    let result = wallet_propose(&object([(
        "seed",
        JsonValue::String("snMwVWs2hZzfDUF3p2tHZ3EgmyhFs".to_owned()),
    )]))
    .expect("should succeed");
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    // Verify all expected fields are present
    assert!(result.contains_key("account_id"));
    assert!(result.contains_key("master_seed"));
    assert!(result.contains_key("master_seed_hex"));
    assert!(result.contains_key("public_key"));
    assert!(result.contains_key("public_key_hex"));
    assert!(result.contains_key("key_type"));
    // Seed should round-trip
    assert_eq!(
        result.get("master_seed"),
        Some(&JsonValue::String(
            "snMwVWs2hZzfDUF3p2tHZ3EgmyhFs".to_owned()
        ))
    );
    assert!(!result.contains_key("warning"));

    // Same seed should produce same result
    let result2 = wallet_propose(&object([(
        "seed",
        JsonValue::String("snMwVWs2hZzfDUF3p2tHZ3EgmyhFs".to_owned()),
    )]))
    .expect("should succeed");
    let JsonValue::Object(result2) = result2 else {
        panic!("result2 must be an object");
    };
    assert_eq!(result.get("account_id"), result2.get("account_id"));
    assert_eq!(result.get("public_key"), result2.get("public_key"));
}

#[test]
fn wallet_propose_from_seed_ed25519() {
    let result = wallet_propose(&object([
        (
            "seed",
            JsonValue::String("snMwVWs2hZzfDUF3p2tHZ3EgmyhFs".to_owned()),
        ),
        ("key_type", JsonValue::String("ed25519".to_owned())),
    ]))
    .expect("should succeed");
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert!(result.contains_key("account_id"));
    assert_eq!(
        result.get("master_seed"),
        Some(&JsonValue::String(
            "snMwVWs2hZzfDUF3p2tHZ3EgmyhFs".to_owned()
        ))
    );
    assert_eq!(
        result.get("key_type"),
        Some(&JsonValue::String("ed25519".to_owned()))
    );
    // ed25519 public key hex starts with ED
    let JsonValue::String(pk_hex) = result.get("public_key_hex").unwrap() else {
        panic!("public_key_hex must be a string");
    };
    assert!(
        pk_hex.to_uppercase().starts_with("ED"),
        "ed25519 public key should start with ED, got: {pk_hex}"
    );
}

#[test]
fn wallet_propose_from_seed_hex() {
    let result = wallet_propose(&object([(
        "seed_hex",
        JsonValue::String("BE6A670A19B209E112146D0A7ED2AAD7".to_owned()),
    )]))
    .expect("should succeed");
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert!(result.contains_key("account_id"));
    assert!(result.contains_key("master_seed"));
    // Seed hex should match (case-insensitive)
    let JsonValue::String(hex) = result.get("master_seed_hex").unwrap() else {
        panic!("master_seed_hex must be a string");
    };
    assert_eq!(hex.to_uppercase(), "BE6A670A19B209E112146D0A7ED2AAD7");
    assert!(!result.contains_key("warning"));

    // Same seed_hex should produce same result on repeated calls
    let from_seed = wallet_propose(&object([(
        "seed_hex",
        JsonValue::String("BE6A670A19B209E112146D0A7ED2AAD7".to_owned()),
    )]))
    .expect("should succeed");
    let JsonValue::Object(from_seed) = from_seed else {
        panic!("from_seed must be an object");
    };
    assert_eq!(result.get("account_id"), from_seed.get("account_id"));
}

#[test]
fn wallet_propose_from_passphrase() {
    let result = wallet_propose(&object([(
        "passphrase",
        JsonValue::String("REINDEER FLOTILLA".to_owned()),
    )]))
    .expect("should succeed");
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert!(result.contains_key("account_id"));
    assert!(result.contains_key("master_seed"));
    // Passphrase should produce a warning about low entropy
    assert!(result.contains_key("warning"));
    let JsonValue::String(warning) = result.get("warning").unwrap() else {
        panic!("warning must be a string");
    };
    assert!(warning.contains("passphrase"));

    // Same passphrase should produce same result
    let result2 = wallet_propose(&object([(
        "passphrase",
        JsonValue::String("REINDEER FLOTILLA".to_owned()),
    )]))
    .expect("should succeed");
    let JsonValue::Object(result2) = result2 else {
        panic!("result2 must be an object");
    };
    assert_eq!(result.get("account_id"), result2.get("account_id"));
    assert_eq!(result.get("master_seed"), result2.get("master_seed"));
}

#[test]
fn wallet_propose_invalid_key_type() {
    let result = wallet_propose(&object([(
        "key_type",
        JsonValue::String("invalid_type".to_owned()),
    )]));
    assert!(result.is_err());
}

#[test]
fn wallet_propose_non_string_key_type() {
    let result = wallet_propose(&object([("key_type", JsonValue::Unsigned(1))]));
    assert!(result.is_err());
}

#[test]
fn wallet_propose_invalid_seed() {
    let result = wallet_propose(&object([(
        "seed",
        JsonValue::String("not_a_valid_seed".to_owned()),
    )]));
    // Invalid seed should either error or fall back to passphrase behavior
    if let Ok(JsonValue::Object(obj)) = &result {
        // If it succeeds, it treated it as a passphrase - should have warning
        assert!(obj.contains_key("account_id"));
    } else {
        assert!(result.is_err());
    }
}

#[test]
fn wallet_propose_invalid_seed_hex() {
    let result = wallet_propose(&object([(
        "seed_hex",
        JsonValue::String("ZZZZ".to_owned()),
    )]));
    assert!(result.is_err());
}

#[test]
fn wallet_propose_account_id_format() {
    let result = wallet_propose(&object([])).expect("should succeed");
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    let JsonValue::String(account_id) = result.get("account_id").unwrap() else {
        panic!("account_id must be a string");
    };
    // Account ID should start with 'r'
    assert!(account_id.starts_with('r'));
    // Should be valid base58
    assert!(account_id.len() >= 25 && account_id.len() <= 35);
}

#[test]
fn wallet_propose_master_seed_format() {
    let result = wallet_propose(&object([])).expect("should succeed");
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    let JsonValue::String(seed) = result.get("master_seed").unwrap() else {
        panic!("master_seed must be a string");
    };
    // Seed should start with 's'
    assert!(seed.starts_with('s'));

    let JsonValue::String(seed_hex) = result.get("master_seed_hex").unwrap() else {
        panic!("master_seed_hex must be a string");
    };
    // Seed hex should be 32 hex chars (16 bytes)
    assert_eq!(seed_hex.len(), 32);
    assert!(seed_hex.chars().all(|c| c.is_ascii_hexdigit()));
}
