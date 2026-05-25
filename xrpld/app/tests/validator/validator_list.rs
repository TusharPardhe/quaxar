use std::collections::HashSet;
use std::path::PathBuf;

use app::{
    ListDisposition, ManifestCache, ManifestDisposition, PublisherStatus, TrustChanges,
    ValidatorBlobInfo, ValidatorList, ValidatorListClock, validator_list_collection_hash,
};
use basics::base_uint::Uint256;
use basics::base64::base64_encode;
use basics::chrono::{NetClockTimePoint, to_string};
use basics::str_hex::str_hex;
use protocol::{
    HashPrefix, JsonValue, KeyType, PublicKey, SField, STObject, STValidation, SecretKey,
    Serializer, StBase, VF_FULL_VALIDATION, calc_account_id, calc_node_id, derive_public_key,
    get_field_by_symbol, sign,
};

#[derive(Clone, Copy, Debug)]
struct FixedClock {
    now_ripple: u32,
}

impl ValidatorListClock for FixedClock {
    fn now_ripple(&self) -> u32 {
        self.now_ripple
    }
}

fn temp_data_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("xrpl-validator-list-parity-{name}"))
}

fn manifest_blob(
    master_secret: &SecretKey,
    master_type: KeyType,
    signing_secret: &SecretKey,
    signing_type: KeyType,
    sequence: u32,
) -> (String, PublicKey, PublicKey) {
    let master_public = derive_public_key(master_type, master_secret).expect("master key");
    let signing_public = derive_public_key(signing_type, signing_secret).expect("signing key");

    let mut object = STObject::new(protocol::sf_generic());
    object.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    object.set_field_vl(get_field_by_symbol("sfPublicKey"), master_public.as_bytes());
    object.set_field_vl(
        get_field_by_symbol("sfSigningPubKey"),
        signing_public.as_bytes(),
    );

    set_manifest_signature(
        &mut object,
        &signing_public,
        signing_secret,
        get_field_by_symbol("sfSignature"),
    );
    set_manifest_signature(
        &mut object,
        &master_public,
        master_secret,
        get_field_by_symbol("sfMasterSignature"),
    );

    let mut serializer = Serializer::default();
    object.add(&mut serializer);
    (
        base64_encode(serializer.data()),
        master_public,
        signing_public,
    )
}

fn set_manifest_signature(
    object: &mut STObject,
    public_key: &PublicKey,
    secret_key: &SecretKey,
    field: &'static SField,
) {
    let mut serializer = Serializer::default();
    serializer.add32_prefix(HashPrefix::Manifest);
    object.add_without_signing_fields(&mut serializer);
    let signature = sign(public_key, secret_key, serializer.data()).expect("manifest signature");
    object.set_field_vl(field, &signature);
}

fn sign_list(blob: &str, signing_public: &PublicKey, signing_secret: &SecretKey) -> String {
    let signature = sign(
        signing_public,
        signing_secret,
        &basics::base64::base64_decode(blob),
    )
    .expect("list signature");
    str_hex(signature)
}

fn build_validator_list_blob(
    sequence: u64,
    expiration: u64,
    validator_master: PublicKey,
    validator_manifest: &str,
) -> String {
    base64_encode(
        serde_json::json!({
            "sequence": sequence,
            "expiration": expiration,
            "validators": [{
                "validation_public_key": validator_master.to_hex(),
                "manifest": validator_manifest,
            }],
        })
        .to_string()
        .as_bytes(),
    )
}

#[test]
fn trusted_publisher_acceptance_surfaces_listed_and_trusted_public_api_output() {
    let clock = FixedClock { now_ripple: 10_000 };
    let list = ValidatorList::new(
        ManifestCache::new(),
        ManifestCache::new(),
        clock,
        temp_data_path("acceptance"),
        None,
    );

    let publisher_master_secret = SecretKey::from_bytes([3u8; 32]);
    let publisher_signing_secret = SecretKey::from_bytes([4u8; 32]);
    let (publisher_manifest, publisher_master, publisher_signing) = manifest_blob(
        &publisher_master_secret,
        KeyType::Ed25519,
        &publisher_signing_secret,
        KeyType::Secp256k1,
        1,
    );

    assert!(list.load(None, &[], &[publisher_master.to_hex()], None));
    assert!(list.trusted_publisher(publisher_master));
    assert_eq!(list.local_public_key(), None);
    assert_eq!(list.get_list_threshold(), 1);

    let validator_master_secret = SecretKey::from_bytes([5u8; 32]);
    let validator_signing_secret = SecretKey::from_bytes([6u8; 32]);
    let (validator_manifest, validator_master, validator_signing) = manifest_blob(
        &validator_master_secret,
        KeyType::Ed25519,
        &validator_signing_secret,
        KeyType::Secp256k1,
        1,
    );

    let blob = build_validator_list_blob(
        2,
        u64::from(clock.now_ripple()) + 3600,
        validator_master,
        &validator_manifest,
    );
    let signature = sign_list(&blob, &publisher_signing, &publisher_signing_secret);
    let blobs = vec![ValidatorBlobInfo {
        blob,
        signature,
        manifest: None,
    }];
    let hash = validator_list_collection_hash(&publisher_manifest, 1, &blobs);

    let result = list.apply_lists(
        &publisher_manifest,
        1,
        &blobs,
        "file:///tmp/validator-list.json".to_owned(),
        Some(hash),
    );

    assert_eq!(result.best_disposition(), ListDisposition::Accepted);
    assert_eq!(result.worst_disposition(), ListDisposition::Accepted);
    assert_eq!(result.publisher_key, Some(publisher_master));
    assert_eq!(result.status, PublisherStatus::Available);
    assert_eq!(result.sequence, 2);

    assert!(list.listed(validator_master));
    assert!(list.listed(validator_signing));
    assert_eq!(
        list.get_listed_key(validator_master),
        Some(validator_master)
    );
    assert_eq!(
        list.get_listed_key(validator_signing),
        Some(validator_master)
    );
    assert!(!list.trusted(validator_master));
    assert!(!list.trusted(validator_signing));

    let changes = list.update_trusted(&HashSet::new(), clock.now_ripple());
    assert_eq!(
        changes,
        TrustChanges {
            added: [calc_account_id(validator_master.as_bytes())]
                .into_iter()
                .collect(),
            removed: HashSet::new().into_iter().collect(),
        }
    );

    assert!(list.trusted(validator_master));
    assert!(list.trusted(validator_signing));
    assert_eq!(
        list.get_trusted_key(validator_master),
        Some(validator_master)
    );
    assert_eq!(
        list.get_trusted_key(validator_signing),
        Some(validator_master)
    );
    assert_eq!(
        list.get_trusted_master_keys(),
        [validator_master].into_iter().collect()
    );
    assert_eq!(list.quorum(), 1);
    assert_eq!(list.expires(), Some(clock.now_ripple() + 3600));

    let mut validation = STValidation::new_signed(
        1,
        &validator_signing,
        calc_node_id(&validator_master),
        &validator_signing_secret,
        |validation| {
            validation.set_field_h256(get_field_by_symbol("sfLedgerHash"), Uint256::from_u64(1));
            validation.set_field_u32(get_field_by_symbol("sfLedgerSequence"), 2);
            validation.set_flag(VF_FULL_VALIDATION);
        },
    )
    .expect("validation should sign");
    validation.set_trusted();
    assert_eq!(
        list.negative_unl_filter_validations(vec![validation.clone()])
            .len(),
        1
    );
    list.set_negative_unl([validator_master].into_iter().collect());
    assert!(
        list.negative_unl_filter_validations(vec![validation])
            .is_empty()
    );

    let mut listed = Vec::new();
    list.for_each_listed(|public_key, trusted| listed.push((public_key, trusted)));
    assert_eq!(listed, vec![(validator_master, true)]);

    let json = list.get_json();
    match json {
        JsonValue::Object(fields) => {
            assert_eq!(fields.get("list_threshold"), Some(&JsonValue::Unsigned(1)));
            assert_eq!(fields.get("quorum"), Some(&JsonValue::Unsigned(1)));
            assert_eq!(
                fields.get("validation_quorum"),
                Some(&JsonValue::Unsigned(1))
            );
            let JsonValue::Object(summary) = fields
                .get("validator_list")
                .expect("validator_list summary should exist")
            else {
                panic!("validator_list should be object");
            };
            assert_eq!(summary.get("count"), Some(&JsonValue::Unsigned(1)));
            assert_eq!(
                summary.get("expiration"),
                Some(&JsonValue::String(to_string(NetClockTimePoint::new(
                    clock.now_ripple() + 3600
                ))))
            );
            assert_eq!(
                summary.get("status"),
                Some(&JsonValue::String("active".to_owned()))
            );
            assert_eq!(
                summary.get("validator_list_threshold"),
                Some(&JsonValue::Unsigned(1))
            );
            assert_eq!(
                fields.get("local_static_keys"),
                Some(&JsonValue::Array(Vec::new()))
            );
            assert_eq!(
                fields.get("trusted_validator_keys"),
                Some(&JsonValue::Array(vec![JsonValue::String(
                    validator_master.to_node_public_base58()
                )]))
            );
        }
        other => panic!("expected object json, got {other:?}"),
    }
}

#[test]
fn trusted_publisher_bad_signature_is_rejected_without_listing_validator() {
    let clock = FixedClock { now_ripple: 20_000 };
    let list = ValidatorList::new(
        ManifestCache::new(),
        ManifestCache::new(),
        clock,
        temp_data_path("bad-signature"),
        None,
    );

    let publisher_master_secret = SecretKey::from_bytes([13u8; 32]);
    let publisher_signing_secret = SecretKey::from_bytes([14u8; 32]);
    let (publisher_manifest, publisher_master, publisher_signing) = manifest_blob(
        &publisher_master_secret,
        KeyType::Ed25519,
        &publisher_signing_secret,
        KeyType::Secp256k1,
        1,
    );
    assert!(list.load(None, &[], &[publisher_master.to_hex()], None));
    assert!(list.trusted_publisher(publisher_master));

    let validator_master_secret = SecretKey::from_bytes([15u8; 32]);
    let validator_signing_secret = SecretKey::from_bytes([16u8; 32]);
    let (validator_manifest, validator_master, _) = manifest_blob(
        &validator_master_secret,
        KeyType::Ed25519,
        &validator_signing_secret,
        KeyType::Secp256k1,
        1,
    );

    let blob = build_validator_list_blob(
        3,
        u64::from(clock.now_ripple()) + 3600,
        validator_master,
        &validator_manifest,
    );
    let mut bad_signature = sign_list(&blob, &publisher_signing, &publisher_signing_secret);
    bad_signature.push('0');
    let blobs = vec![ValidatorBlobInfo {
        blob,
        signature: bad_signature,
        manifest: None,
    }];

    let result = list.apply_lists(
        &publisher_manifest,
        1,
        &blobs,
        "file:///tmp/validator-list.json".to_owned(),
        None,
    );

    assert_eq!(result.best_disposition(), ListDisposition::Invalid);
    assert_eq!(result.publisher_key, Some(publisher_master));
    assert_eq!(result.status, PublisherStatus::Unavailable);
    assert!(!list.listed(validator_master));
    assert!(!list.trusted(validator_master));
    assert_eq!(
        list.update_trusted(&HashSet::new(), clock.now_ripple()),
        TrustChanges::default()
    );
}

#[test]
fn unfetched_validator_list_reports_unknown_expiration() {
    let clock = FixedClock { now_ripple: 30_000 };
    let list = ValidatorList::new(
        ManifestCache::new(),
        ManifestCache::new(),
        clock,
        temp_data_path("unknown-expiration"),
        None,
    );

    let publisher_master_secret = SecretKey::from_bytes([25u8; 32]);
    let publisher_signing_secret = SecretKey::from_bytes([26u8; 32]);
    let (_publisher_manifest, publisher_master, _publisher_signing) = manifest_blob(
        &publisher_master_secret,
        KeyType::Ed25519,
        &publisher_signing_secret,
        KeyType::Secp256k1,
        1,
    );
    assert!(list.load(None, &[], &[publisher_master.to_hex()], None));

    assert_eq!(list.count(), 1);
    assert_eq!(list.expires(), None);

    let JsonValue::Object(fields) = list.get_json() else {
        panic!("validator list json should be object");
    };
    let JsonValue::Object(summary) = fields
        .get("validator_list")
        .expect("validator_list summary should exist")
    else {
        panic!("validator_list summary should be object");
    };
    assert_eq!(summary.get("count"), Some(&JsonValue::Unsigned(1)));
    assert_eq!(
        summary.get("status"),
        Some(&JsonValue::String("unknown".to_owned()))
    );
    assert_eq!(
        summary.get("expiration"),
        Some(&JsonValue::String("unknown".to_owned()))
    );
}

#[test]
fn untrusted_publisher_valid_manifest_is_rejected() {
    let clock = FixedClock { now_ripple: 30_000 };
    let list = ValidatorList::new(
        ManifestCache::new(),
        ManifestCache::new(),
        clock,
        temp_data_path("untrusted"),
        None,
    );

    let publisher_master_secret = SecretKey::from_bytes([23u8; 32]);
    let publisher_signing_secret = SecretKey::from_bytes([24u8; 32]);
    let (publisher_manifest, publisher_master, publisher_signing) = manifest_blob(
        &publisher_master_secret,
        KeyType::Ed25519,
        &publisher_signing_secret,
        KeyType::Secp256k1,
        1,
    );

    let validator_master_secret = SecretKey::from_bytes([25u8; 32]);
    let validator_signing_secret = SecretKey::from_bytes([26u8; 32]);
    let (validator_manifest, validator_master, _) = manifest_blob(
        &validator_master_secret,
        KeyType::Ed25519,
        &validator_signing_secret,
        KeyType::Secp256k1,
        1,
    );

    let blob = build_validator_list_blob(
        4,
        u64::from(clock.now_ripple()) + 3600,
        validator_master,
        &validator_manifest,
    );
    let signature = sign_list(&blob, &publisher_signing, &publisher_signing_secret);
    let blobs = vec![ValidatorBlobInfo {
        blob,
        signature,
        manifest: None,
    }];

    let result = list.apply_lists(
        &publisher_manifest,
        1,
        &blobs,
        "file:///tmp/validator-list.json".to_owned(),
        None,
    );

    assert_eq!(result.best_disposition(), ListDisposition::Untrusted);
    assert_eq!(result.status, PublisherStatus::Unavailable);
    assert_eq!(result.publisher_key, None);
    assert!(!list.trusted_publisher(publisher_master));
    assert!(!list.listed(validator_master));
    assert!(!list.trusted(validator_master));
    assert_eq!(
        list.update_trusted(&HashSet::new(), clock.now_ripple()),
        TrustChanges::default()
    );
}

#[test]
fn parse_blobs_public_api_matches_v1_and_v2_shapes() {
    let v1 = serde_json::json!({
        "blob": "Zm9v",
        "signature": "DEADBEEF",
        "version": 1,
    });
    let v1_blobs = ValidatorList::<FixedClock>::parse_blobs(1, &v1);
    assert_eq!(v1_blobs.len(), 1);
    assert_eq!(v1_blobs[0].blob, "Zm9v");
    assert_eq!(v1_blobs[0].signature, "DEADBEEF");
    assert_eq!(v1_blobs[0].manifest, None);

    let v2 = serde_json::json!({
        "version": 2,
        "blobs_v2": [
            {"blob": "Zm9v", "signature": "DEADBEEF", "manifest": "bWFuaWZlc3Q="},
            {"blob": "YmFy", "signature": "CAFEBABE"},
        ],
    });
    let v2_blobs = ValidatorList::<FixedClock>::parse_blobs(2, &v2);
    assert_eq!(v2_blobs.len(), 2);
    assert_eq!(v2_blobs[0].manifest.as_deref(), Some("bWFuaWZlc3Q="));

    let invalid = serde_json::json!({
        "blob": "Zm9v",
        "signature": "DEADBEEF",
        "blobs_v2": [],
    });
    assert!(ValidatorList::<FixedClock>::parse_blobs(1, &invalid).is_empty());
}

#[test]
fn parse_blobs_rejects_oversized_v2_collection() {
    let entries = (0..6)
        .map(|_| serde_json::json!({"blob": "Zm9v", "signature": "DEADBEEF"}))
        .collect::<Vec<_>>();
    let invalid = serde_json::json!({
        "version": 2,
        "blobs_v2": entries,
    });
    assert!(ValidatorList::<FixedClock>::parse_blobs(2, &invalid).is_empty());
    assert_eq!(ManifestDisposition::Accepted as u8, 0);
}
