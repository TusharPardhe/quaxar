#[path = "../../src/state/manifest.rs"]
mod manifest;

use basics::base64::base64_encode;
use manifest::{
    Manifest, ManifestCache, ManifestDisposition, deserialize_manifest, load_validator_token,
};
use protocol::{
    HashPrefix, KeyType, PublicKey, SField, STObject, SecretKey, Serializer, derive_public_key,
    get_field_by_symbol, sf_generic, sign,
};

fn signing_bytes(st: &STObject) -> Vec<u8> {
    let mut serializer = Serializer::default();
    serializer.add32_prefix(HashPrefix::Manifest);
    st.add_without_signing_fields(&mut serializer);
    serializer.data().to_vec()
}

fn set_signature(
    st: &mut STObject,
    public_key: &PublicKey,
    secret_key: &SecretKey,
    field: &'static SField,
) {
    let signature =
        sign(public_key, secret_key, &signing_bytes(st)).expect("signature should be created");
    st.set_field_vl(field, &signature);
}

fn serialize(st: &STObject) -> Vec<u8> {
    st.get_serializer().data().to_vec()
}

fn secret(key_type: KeyType, fill: u8) -> SecretKey {
    let mut bytes = [fill; 32];
    if matches!(key_type, KeyType::Secp256k1) {
        bytes[31] = fill.max(1);
    }
    SecretKey::from_bytes(bytes)
}

#[allow(clippy::too_many_arguments)]
fn build_manifest_object(
    master_type: KeyType,
    master_fill: u8,
    signing_type: KeyType,
    signing_fill: u8,
    sequence: u32,
    domain: Option<&str>,
    include_signing_key: bool,
    include_signing_signature: bool,
) -> (STObject, PublicKey, SecretKey, PublicKey, SecretKey) {
    let master_secret = secret(master_type, master_fill);
    let master_public =
        derive_public_key(master_type, &master_secret).expect("master public key should derive");
    let signing_secret = secret(signing_type, signing_fill);
    let signing_public =
        derive_public_key(signing_type, &signing_secret).expect("signing public key should derive");

    let mut st = STObject::new(sf_generic());
    st.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    st.set_field_vl(get_field_by_symbol("sfPublicKey"), master_public.as_bytes());
    if let Some(domain) = domain {
        st.set_field_vl(get_field_by_symbol("sfDomain"), domain.as_bytes());
    }
    if include_signing_key {
        st.set_field_vl(
            get_field_by_symbol("sfSigningPubKey"),
            signing_public.as_bytes(),
        );
    }
    if include_signing_signature {
        set_signature(
            &mut st,
            &signing_public,
            &signing_secret,
            get_field_by_symbol("sfSignature"),
        );
    }
    set_signature(
        &mut st,
        &master_public,
        &master_secret,
        get_field_by_symbol("sfMasterSignature"),
    );

    (
        st,
        master_public,
        master_secret,
        signing_public,
        signing_secret,
    )
}

fn build_manifest(
    master_type: KeyType,
    master_fill: u8,
    signing_type: KeyType,
    signing_fill: u8,
    sequence: u32,
    domain: Option<&str>,
) -> Manifest {
    let (st, ..) = build_manifest_object(
        master_type,
        master_fill,
        signing_type,
        signing_fill,
        sequence,
        domain,
        true,
        true,
    );
    deserialize_manifest(&serialize(&st)).expect("manifest should deserialize")
}

fn build_revocation(master_type: KeyType, master_fill: u8) -> Manifest {
    let (st, ..) = build_manifest_object(
        master_type,
        master_fill,
        KeyType::Secp256k1,
        88,
        u32::MAX,
        None,
        false,
        false,
    );
    deserialize_manifest(&serialize(&st)).expect("revocation should deserialize")
}

#[test]
fn deserialize_and_verify_manifest_rules() {
    for (master_type, signing_type) in [
        (KeyType::Ed25519, KeyType::Secp256k1),
        (KeyType::Secp256k1, KeyType::Ed25519),
    ] {
        let manifest = build_manifest(master_type, 11, signing_type, 21, 7, Some("example.com"));
        assert_eq!(manifest.sequence, 7);
        assert_eq!(manifest.domain, "example.com");
        assert!(manifest.signing_key.is_some());
        assert!(manifest.verify());
        assert_eq!(
            manifest.hash(),
            Some(
                deserialize_manifest(&manifest.serialized)
                    .expect("manifest should round-trip")
                    .hash()
                    .expect("hash should exist")
            )
        );
        assert!(manifest.get_signature().is_some());
        assert!(manifest.get_master_signature().is_some());

        let (mut bad_version, ..) =
            build_manifest_object(master_type, 12, signing_type, 22, 8, None, true, true);
        bad_version.set_field_u16(get_field_by_symbol("sfVersion"), 1);
        assert!(deserialize_manifest(&serialize(&bad_version)).is_none());

        let (mut bad_sig, master_public, ..) = build_manifest_object(
            master_type,
            13,
            signing_type,
            23,
            9,
            Some("example.com"),
            true,
            true,
        );
        bad_sig.set_field_u32(get_field_by_symbol("sfSequence"), 10);
        let bad_sig =
            deserialize_manifest(&serialize(&bad_sig)).expect("tampered manifest should parse");
        assert_eq!(bad_sig.master_key, master_public);
        assert!(!bad_sig.verify());
    }
}

#[test]
fn deserialize_manifest_rejects_invalid_domain_and_revocation_shapes() {
    let (empty_domain, ..) = build_manifest_object(
        KeyType::Ed25519,
        31,
        KeyType::Secp256k1,
        41,
        3,
        Some(""),
        true,
        true,
    );
    assert!(deserialize_manifest(&serialize(&empty_domain)).is_none());

    let (short_domain, ..) = build_manifest_object(
        KeyType::Ed25519,
        32,
        KeyType::Secp256k1,
        42,
        4,
        Some("a.b"),
        true,
        true,
    );
    assert!(deserialize_manifest(&serialize(&short_domain)).is_none());

    let revocation = build_revocation(KeyType::Ed25519, 33);
    assert!(revocation.revoked());
    assert!(revocation.signing_key.is_none());
    assert!(revocation.verify());

    let (bad_revocation_key, ..) = build_manifest_object(
        KeyType::Ed25519,
        34,
        KeyType::Secp256k1,
        44,
        u32::MAX,
        None,
        true,
        false,
    );
    assert!(deserialize_manifest(&serialize(&bad_revocation_key)).is_none());

    let (bad_revocation_sig, ..) = build_manifest_object(
        KeyType::Ed25519,
        35,
        KeyType::Secp256k1,
        45,
        u32::MAX,
        None,
        false,
        true,
    );
    assert!(deserialize_manifest(&serialize(&bad_revocation_sig)).is_none());
}

#[test]
fn manifest_cache_accepts_updates_and_rejects_alias_conflicts() {
    let cache = ManifestCache::new();

    let a0 = build_manifest(KeyType::Ed25519, 51, KeyType::Secp256k1, 61, 0, None);
    let a1 = build_manifest(KeyType::Ed25519, 51, KeyType::Secp256k1, 62, 1, None);
    let a2 = build_manifest(KeyType::Ed25519, 51, KeyType::Secp256k1, 62, 2, None);
    let a_revoked = build_revocation(KeyType::Ed25519, 51);

    let sequence0 = cache.sequence();
    assert_eq!(
        cache.apply_manifest(a0.clone()),
        ManifestDisposition::Accepted
    );
    assert!(cache.sequence() > sequence0);
    assert_eq!(cache.apply_manifest(a0.clone()), ManifestDisposition::Stale);
    assert_eq!(
        cache.get_signing_key(&a0.master_key),
        a0.signing_key.expect("signing key")
    );
    assert_eq!(
        cache.get_master_key(&a0.signing_key.expect("signing key")),
        a0.master_key
    );
    assert_eq!(cache.get_sequence(&a0.master_key), Some(0));
    assert_eq!(cache.get_domain(&a0.master_key), Some(String::new()));
    assert_eq!(
        cache.get_manifest(&a0.master_key),
        Some(a0.serialized.clone())
    );

    assert_eq!(
        cache.apply_manifest(a1.clone()),
        ManifestDisposition::Accepted
    );
    assert_eq!(cache.apply_manifest(a1.clone()), ManifestDisposition::Stale);
    assert_eq!(
        cache.apply_manifest(a2.clone()),
        ManifestDisposition::BadEphemeralKey
    );
    assert_eq!(
        cache.get_signing_key(&a1.master_key),
        a1.signing_key.expect("signing key")
    );
    assert_eq!(
        cache.get_master_key(&a0.signing_key.expect("signing key")),
        a0.signing_key.expect("signing key")
    );

    assert!(!cache.revoked(&a1.master_key));
    assert_eq!(
        cache.apply_manifest(a_revoked.clone()),
        ManifestDisposition::Accepted
    );
    assert_eq!(
        cache.apply_manifest(a_revoked.clone()),
        ManifestDisposition::Stale
    );
    assert_eq!(cache.apply_manifest(a1.clone()), ManifestDisposition::Stale);
    assert!(cache.revoked(&a1.master_key));
    assert_eq!(cache.get_signing_key(&a1.master_key), a1.master_key);
    assert_eq!(
        cache.get_master_key(&a1.signing_key.expect("signing key")),
        a1.signing_key.expect("signing key")
    );
    assert_eq!(cache.get_sequence(&a1.master_key), None);
    assert_eq!(cache.get_domain(&a1.master_key), None);
    assert_eq!(cache.get_manifest(&a1.master_key), None);

    let b0 = build_manifest(KeyType::Ed25519, 71, KeyType::Secp256k1, 81, 0, None);
    let mut b1 = build_manifest(KeyType::Ed25519, 71, KeyType::Secp256k1, 82, 1, None);
    let b2 = build_manifest(KeyType::Ed25519, 71, KeyType::Ed25519, 83, 2, None);
    b1.serialized.push(0);
    assert!(deserialize_manifest(&b1.serialized).is_none());

    assert_eq!(
        cache.apply_manifest(b0.clone()),
        ManifestDisposition::Accepted
    );

    let (mut bad_signature_object, ..) = build_manifest_object(
        KeyType::Ed25519,
        71,
        KeyType::Secp256k1,
        82,
        1,
        None,
        true,
        true,
    );
    bad_signature_object.set_field_u32(get_field_by_symbol("sfSequence"), 2);
    let bad_signature_manifest = deserialize_manifest(&serialize(&bad_signature_object))
        .expect("tampered manifest should parse");
    assert_eq!(
        cache.apply_manifest(bad_signature_manifest),
        ManifestDisposition::Invalid
    );
    assert_eq!(
        cache.apply_manifest(b2.clone()),
        ManifestDisposition::Accepted
    );

    let c0 = build_manifest(KeyType::Ed25519, 83, KeyType::Ed25519, 84, 47, None);
    assert_eq!(cache.apply_manifest(c0), ManifestDisposition::BadMasterKey);
}

#[test]
fn validator_token_loader_trims_and_decodes_cpp_vector() {
    let token_blob = vec![
        "    eyJ2YWxpZGF0aW9uX3NlY3JldF9rZXkiOiI5ZWQ0NWY4NjYyNDFjYzE4YTI3NDdiNT".to_owned(),
        " \tQzODdjMDYyNTkwNzk3MmY0ZTcxOTAyMzFmYWE5Mzc0NTdmYTlkYWY2IiwibWFuaWZl     ".to_owned(),
        "\tc3QiOiJKQUFBQUFGeEllMUZ0d21pbXZHdEgyaUNjTUpxQzlnVkZLaWxHZncxL3ZDeE".to_owned(),
        "\t hYWExwbGMyR25NaEFrRTFhZ3FYeEJ3RHdEYklENk9NU1l1TTBGREFscEFnTms4U0tG\t  \t".to_owned(),
        "bjdNTzJmZGtjd1JRSWhBT25ndTlzQUtxWFlvdUorbDJWMFcrc0FPa1ZCK1pSUzZQU2".to_owned(),
        "hsSkFmVXNYZkFpQnNWSkdlc2FhZE9KYy9hQVpva1MxdnltR21WcmxIUEtXWDNZeXd1".to_owned(),
        "NmluOEhBU1FLUHVnQkQ2N2tNYVJGR3ZtcEFUSGxHS0pkdkRGbFdQWXk1QXFEZWRGdj".to_owned(),
        "VUSmEydzBpMjFlcTNNWXl3TFZKWm5GT3I3QzBrdzJBaVR6U0NqSXpkaXRROD0ifQ==".to_owned(),
    ];

    let token = load_validator_token(token_blob).expect("validator token should decode");
    assert_eq!(
        token.manifest,
        "JAAAAAFxIe1FtwmimvGtH2iCcMJqC9gVFKilGfw1/vCxHXXLplc2GnMhAkE1agqXxBwDwDbID6OMSYuM0FDAlpAgNk8SKFn7MO2fdkcwRQIhAOngu9sAKqXYouJ+l2V0W+sAOkVB+ZRS6PShlJAfUsXfAiBsVJGesaadOJc/aAZokS1vymGmVrlHPKWX3Yywu6in8HASQKPugBD67kMaRFGvmpATHlGKJdvDFlWPYy5AqDedFv5TJa2w0i21eq3MYywLVJZnFOr7C0kw2AiTzSCjIzditQ8="
    );
    assert_eq!(
        token.validation_secret.to_hex(),
        "9ED45F866241CC18A2747B54387C0625907972F4E7190231FAA937457FA9DAF6"
    );

    let reencoded = base64_encode(token.manifest.as_bytes());
    assert!(!reencoded.is_empty());
    assert!(load_validator_token(vec!["bad token".to_owned()]).is_none());
}
