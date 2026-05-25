use app::{ValidatorKeys, calc_node_id, deserialize_manifest, parse_node_private_base58};
use basics::base64::base64_decode;
use protocol::{KeyType, derive_public_key, generate_secret_key, parse_base58_seed};

#[test]
fn validator_keys_match_cpp_seed_and_token_vectors() {
    let no_config = ValidatorKeys::from_sources(None, None);
    assert!(no_config.keys.is_none());
    assert!(no_config.manifest.is_empty());
    assert!(!no_config.config_invalid());

    let seed = "shUwVw52ofnCUX5m7kPTKzJdr4HEH";
    let from_seed = ValidatorKeys::from_sources(Some(seed), None);
    assert!(!from_seed.config_invalid());
    assert!(from_seed.manifest.is_empty());
    assert_eq!(from_seed.sequence, 0);
    let seed_keys = from_seed
        .keys
        .as_ref()
        .expect("validation seed should produce keys");
    assert_eq!(seed_keys.master_public_key, seed_keys.public_key);
    assert_eq!(from_seed.node_id, calc_node_id(&seed_keys.public_key));

    let bad_seed = ValidatorKeys::from_sources(Some("badseed"), None);
    assert!(bad_seed.config_invalid());
    assert!(bad_seed.keys.is_none());
    assert!(bad_seed.manifest.is_empty());

    let token_secret_base58 = "paQmjZ37pKKPMrgadBLsuf9ab7Y7EUNzh27LQrZqoexpAs31nJi";
    let expected_token_secret =
        parse_node_private_base58(token_secret_base58).expect("node private vector should parse");
    let expected_token_public = derive_public_key(KeyType::Secp256k1, &expected_token_secret)
        .expect("token public key should derive");

    let token_manifest = "JAAAAAFxIe1FtwmimvGtH2iCcMJqC9gVFKilGfw1/vCxHXXLplc2GnMhAkE1agqXxBwDwDbID6OMSYuM0FDAlpAgNk8SKFn7MO2fdkcwRQIhAOngu9sAKqXYouJ+l2V0W+sAOkVB+ZRS6PShlJAfUsXfAiBsVJGesaadOJc/aAZokS1vymGmVrlHPKWX3Yywu6in8HASQKPugBD67kMaRFGvmpATHlGKJdvDFlWPYy5AqDedFv5TJa2w0i21eq3MYywLVJZnFOr7C0kw2AiTzSCjIzditQ8=";
    let expected_manifest =
        deserialize_manifest(&base64_decode(token_manifest)).expect("token manifest should parse");

    let token_blob = vec![
        "    eyJ2YWxpZGF0aW9uX3NlY3JldF9rZXkiOiI5ZWQ0NWY4NjYyNDFjYzE4YTI3NDdiNT\n".to_owned(),
        " \tQzODdjMDYyNTkwNzk3MmY0ZTcxOTAyMzFmYWE5Mzc0NTdmYTlkYWY2IiwibWFuaWZl     \n".to_owned(),
        "\tc3QiOiJKQUFBQUFGeEllMUZ0d21pbXZHdEgyaUNjTUpxQzlnVkZLaWxHZncxL3ZDeE\n".to_owned(),
        "\t hYWExwbGMyR25NaEFrRTFhZ3FYeEJ3RHdEYklENk9NU1l1TTBGREFscEFnTms4U0tG\t  \t\n".to_owned(),
        "bjdNTzJmZGtjd1JRSWhBT25ndTlzQUtxWFlvdUorbDJWMFcrc0FPa1ZCK1pSUzZQU2\n".to_owned(),
        "hsSkFmVXNYZkFpQnNWSkdlc2FhZE9KYy9hQVpva1MxdnltR21WcmxIUEtXWDNZeXd1\n".to_owned(),
        "NmluOEhBU1FLUHVnQkQ2N2tNYVJGR3ZtcEFUSGxHS0pkdkRGbFdQWXk1QXFEZWRGdj\n".to_owned(),
        "VUSmEydzBpMjFlcTNNWXl3TFZKWm5GT3I3QzBrdzJBaVR6U0NqSXpkaXRROD0ifQ==\n".to_owned(),
    ];
    let from_token = ValidatorKeys::from_sources(None, Some(&token_blob));
    assert!(!from_token.config_invalid());
    assert_eq!(from_token.manifest, token_manifest);
    assert_eq!(from_token.sequence, expected_manifest.sequence);
    assert_eq!(
        from_token.node_id,
        calc_node_id(&expected_manifest.master_key)
    );
    let token_keys = from_token
        .keys
        .as_ref()
        .expect("validation token should produce keys");
    assert_eq!(token_keys.master_public_key, expected_manifest.master_key);
    assert_eq!(token_keys.public_key, expected_token_public);
    assert_eq!(
        token_keys.secret_key.to_hex(),
        expected_token_secret.to_hex()
    );

    let invalid_token = ValidatorKeys::from_sources(None, Some(&["badtoken".to_owned()]));
    assert!(invalid_token.config_invalid());
    assert!(invalid_token.keys.is_none());
    assert!(invalid_token.manifest.is_empty());

    let both = ValidatorKeys::from_sources(Some(seed), Some(&token_blob));
    assert!(both.config_invalid());
    assert!(both.keys.is_none());
    assert!(both.manifest.is_empty());

    let invalid_token_blob = vec![
        "eyJtYW5pZmVzdCI6IkpBQUFBQVZ4SWUyOVVBdzViZFJudHJ1elVkREk4aDNGV1JWZlk3SXVIaUlKQUhJd3MxdzZzM01oQWtsa1VXQWR2RnFRVGRlSEpvS1pNY0hlS0RzOExob3d3bDlHOEdkVGNJbmFka1l3UkFJZ0h2Q01lQU1aSzlqQnV2aFhlaFRLRzVDQ3BBR1k0bGtvZHRXYW84UGhzR3NDSUREVTA1d1c3bWNiMjlVNkMvTHBpZmgvakZPRGhFR21iNWF6dTJMVHlqL1pjQkpBbitmNGhtQTQ0U0tYbGtTTUFqak1rSWRyR1Rxa21SNjBzVGJaTjZOOUYwdk9UV3VYcUZ6eDFoSGIyL0RqWElVZXhDVGlITEcxTG9UdUp1eXdXbk55RFE9PSIsInZhbGlkYXRpb25fc2VjcmV0X2tleSI6IjkyRDhCNDBGMzYwMTc5MTkwMUMzQTUzMzI3NzBDMkUwMTA4MDI0NTZFOEM2QkI0NEQ0N0FFREQ0NzJGMDQ2RkYifQ==\n".to_owned(),
    ];
    let mismatched = ValidatorKeys::from_sources(None, Some(&invalid_token_blob));
    assert!(mismatched.config_invalid());
    assert!(mismatched.keys.is_none());
}

#[test]
fn local_base58_seed_and_node_private_helpers_match_cpp_reference_vectors() {
    let seed = parse_base58_seed("snoPBrXtMeMyMHUVTgbuqAfg1SUTb")
        .expect("family seed vector should parse");
    let secret =
        generate_secret_key(KeyType::Secp256k1, &seed).expect("family seed should derive secret");
    let public = derive_public_key(KeyType::Secp256k1, &secret)
        .expect("family seed public key should derive");

    assert_eq!(
        public.to_node_public_base58(),
        "n94a1u4jAz288pZLtw6yFWVbi89YamiC6JBXPVUj5zmExe5fTVg9"
    );
    assert_eq!(
        parse_node_private_base58("pnen77YEeUd4fFKG7iycBWcwKpTaeFRkW2WFostaATy1DSupwXe")
            .expect("node private vector should parse")
            .to_hex(),
        secret.to_hex()
    );
    assert_eq!(
        calc_node_id(&public).to_string(),
        "7E59C17D50F5959C7B158FEC95C8F815BF653DC8"
    );
}
