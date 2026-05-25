use basics::{
    base_uint::Uint256,
    number::{NumberRoundModeGuard, RoundingMode},
};
use protocol::{
    Asset, JsonValue, LedgerEntryType, MessageType, STLedgerEntry, STNumber, STParsedJSONObject,
    STPathSet, TmPing, TokenType, associate_asset, calc_account_id, calc_node_id,
    encode_base58_token, generate_seed, get_field_by_symbol, json_static_strings,
    parse_generic_seed, ripesha, seed_as_1751, xrp_issue,
};

#[test]
fn generated_jss_and_messages_surfaces_match_current_cpp_shape() {
    assert_eq!(json_static_strings::ALL.len(), 623);
    assert_eq!(
        json_static_strings::AcceptedCredentials,
        "AcceptedCredentials"
    );
    assert_eq!(json_static_strings::attestations, "attestations");
    assert_eq!(json_static_strings::r#in, "in");
    assert_eq!(json_static_strings::r#type, "type");

    let ping = TmPing::default();
    assert_eq!(ping.r#type, 0);
    assert_eq!(MessageType::MtPing as i32, 3);
    assert_eq!(MessageType::MtPing.as_str_name(), "mtPING");
}

#[test]
fn seed_surface_matches_current_cpp_vectors_and_round_trips() {
    let seed = generate_seed("masterpassphrase");
    assert_eq!(
        encode_base58_token(TokenType::FamilySeed, seed.data()),
        "snoPBrXtMeMyMHUVTgbuqAfg1SUTb"
    );

    let english = seed_as_1751(&seed);
    assert_eq!(parse_generic_seed(&english, true), Some(seed.clone()));

    let invalid = "THIS IS NOT VALID RFC1751";
    assert_eq!(
        parse_generic_seed(invalid, true),
        Some(generate_seed(invalid))
    );
}

#[test]
fn digest_surface_is_adopted_by_account_and_node_id_helpers() {
    let public_key = [
        0x03, 0xD4, 0x9C, 0x56, 0xE1, 0xB1, 0x85, 0xF1, 0xBE, 0x89, 0x9A, 0xE6, 0x6A, 0x02, 0xEF,
        0xC1, 0x7F, 0x78, 0xEA, 0x6F, 0xC5, 0x3A, 0xF8, 0x5E, 0x0F, 0xE5, 0x4C, 0x6E, 0x8B, 0x7F,
        0x8C, 0x71, 0xA8,
    ];
    let public_key = protocol::PublicKey::from_bytes(public_key);

    let digest = ripesha(public_key.as_bytes());
    assert_eq!(
        calc_account_id(public_key.as_bytes()),
        protocol::AccountID::from_slice(&digest).expect("account id width"),
    );
    assert_eq!(
        calc_node_id(&public_key),
        protocol::NodeID::from_slice(&digest).expect("node id width"),
    );
}

#[test]
fn st_takes_asset_rounds_integral_numbers_and_removes_default_fields() {
    let _round = NumberRoundModeGuard::new(RoundingMode::TowardsZero);
    let field = get_field_by_symbol("sfAssetsAvailable");

    let mut vault = STLedgerEntry::from_type_and_key(LedgerEntryType::Vault, Uint256::zero());
    vault.set_field_number(
        field,
        STNumber::with_field(
            field,
            protocol::normalized_parts_from_string("12.9").expect("runtime number"),
        ),
    );
    associate_asset(&mut vault, Asset::Issue(xrp_issue()));
    assert_eq!(vault.get_field_number(field).value().to_string(), "12");

    let mut defaulted = STLedgerEntry::from_type_and_key(LedgerEntryType::Vault, Uint256::zero());
    defaulted.set_field_number(
        field,
        STNumber::with_field(
            field,
            protocol::normalized_parts_from_string("0.9").expect("runtime number"),
        ),
    );
    associate_asset(&mut defaulted, Asset::Issue(xrp_issue()));
    assert!(!defaulted.is_field_present(field));
}

#[test]
fn st_parsed_json_public_surface_parses_pathset_shape() {
    let json = JsonValue::Object(std::collections::BTreeMap::from([(
        "Paths".to_owned(),
        JsonValue::Array(vec![JsonValue::Array(vec![JsonValue::Object(
            std::collections::BTreeMap::from([
                (
                    "account".to_owned(),
                    JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
                ),
                ("currency".to_owned(), JsonValue::String("USD".to_owned())),
                (
                    "issuer".to_owned(),
                    JsonValue::String("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe".to_owned()),
                ),
            ]),
        )])]),
    )]));

    let parsed = STParsedJSONObject::new("test", &json)
        .object
        .expect("pathset should parse");
    let set: STPathSet = parsed.get_field_path_set(get_field_by_symbol("sfPaths"));
    assert_eq!(
        set[0][0].account_id().to_string(),
        "B5F762798A53D543A014CAF8B297CFF8F2F937E8"
    );
    assert_eq!(
        set[0][0].currency().to_string(),
        "0000000000000000000000005553440000000000"
    );
}
