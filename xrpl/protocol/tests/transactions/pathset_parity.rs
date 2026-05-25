use basics::str_hex::str_hex;
use protocol::{
    JsonOptions, JsonValue, PathAsset, STBlob, STPath, STPathElement, STPathSet, STVar, Serializer,
    StBase, currency_from_string, get_field_by_symbol, make_mpt_id, parse_base58_account_id,
    st_path_set_from_json,
};

#[test]
fn pathset_wire_round_trip_matches_current_boundary_and_terminator_rules() {
    let account =
        parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").expect("genesis account");
    let issuer =
        parse_base58_account_id("rrrrrrrrrrrrrrrrrrrrrhoLvTp").expect("zero account base58");
    let currency = currency_from_string("USD");

    let mut first = STPath::new();
    first.push_back(STPathElement::inferred(account, currency, issuer, true));

    let mut second = STPath::new();
    second.push_back(STPathElement::inferred(account, currency, account, true));

    let mut path_set = STPathSet::new(get_field_by_symbol("sfPaths"));
    path_set.push_back(first);
    path_set.push_back(second);

    let mut serializer = Serializer::default();
    path_set.add(&mut serializer);
    assert_eq!(
        str_hex(serializer.data()),
        "11B5F762798A53D543A014CAF8B297CFF8F2F937E80000000000000000000000005553440000000000FF31B5F762798A53D543A014CAF8B297CFF8F2F937E80000000000000000000000005553440000000000B5F762798A53D543A014CAF8B297CFF8F2F937E800"
    );

    let mut iter = protocol::SerialIter::new(serializer.data());
    let parsed = STPathSet::from_serial_iter(&mut iter, get_field_by_symbol("sfPaths"));
    assert_eq!(parsed, path_set);
}

#[test]
fn pathset_parse_rejects_empty_paths_and_bad_element_types() {
    let mut empty = Serializer::default();
    empty.add8(STPathElement::TYPE_NONE);
    let empty_result = std::panic::catch_unwind(|| {
        let mut iter = protocol::SerialIter::new(empty.data());
        let _ = STPathSet::from_serial_iter(&mut iter, get_field_by_symbol("sfPaths"));
    });
    assert!(empty_result.is_err());

    let mut bad = Serializer::default();
    bad.add8(0x02);
    let bad_result = std::panic::catch_unwind(|| {
        let mut iter = protocol::SerialIter::new(bad.data());
        let _ = STPathSet::from_serial_iter(&mut iter, get_field_by_symbol("sfPaths"));
    });
    assert!(bad_result.is_err());
}

#[test]
fn path_element_equality_matches_current_account_bit_rule() {
    let account =
        parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").expect("genesis account");
    let left = STPathElement::raw(
        STPathElement::TYPE_CURRENCY,
        protocol::AccountID::zero(),
        currency_from_string("USD"),
        protocol::AccountID::zero(),
    );
    let right = STPathElement::raw(
        STPathElement::TYPE_CURRENCY | STPathElement::TYPE_ISSUER,
        protocol::AccountID::zero(),
        currency_from_string("USD"),
        protocol::AccountID::zero(),
    );
    let account_path = STPathElement::raw(
        STPathElement::TYPE_ACCOUNT | STPathElement::TYPE_CURRENCY,
        account,
        currency_from_string("USD"),
        protocol::AccountID::zero(),
    );

    assert_eq!(left, right);
    assert_ne!(left, account_path);
}

#[test]
fn pathset_json_helper_matches_current_shape_and_null_rules() {
    let value = JsonValue::Array(vec![
        JsonValue::Array(vec![JsonValue::Object(
            [
                (
                    "account".to_string(),
                    JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_string()),
                ),
                ("currency".to_string(), JsonValue::String("USD".to_string())),
                (
                    "issuer".to_string(),
                    JsonValue::String("0000000000000000000000000000000000000000".to_string()),
                ),
            ]
            .into_iter()
            .collect(),
        )]),
        JsonValue::Null,
    ]);

    let path_set =
        st_path_set_from_json(get_field_by_symbol("sfPaths"), &value).expect("json pathset");
    assert_eq!(path_set.size(), 2);
    assert!(path_set[1].empty());
    assert_eq!(
        path_set.json(JsonOptions::NONE),
        JsonValue::Array(vec![
            JsonValue::Array(vec![JsonValue::Object(
                [
                    ("type".to_string(), JsonValue::Unsigned(17)),
                    (
                        "account".to_string(),
                        JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_string())
                    ),
                    ("currency".to_string(), JsonValue::String("USD".to_string())),
                ]
                .into_iter()
                .collect()
            )]),
            JsonValue::Array(vec![]),
        ])
    );
}

#[test]
fn stvar_now_supports_pathset() {
    let value = STVar::from_serialized_type(
        protocol::SerializedTypeId::PathSet,
        get_field_by_symbol("sfPaths"),
    );
    assert_eq!(value.stype(), protocol::SerializedTypeId::PathSet);
}

#[test]
fn pathset_is_equivalent_value_only_contract() {
    let account =
        parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").expect("genesis account");

    let mut left_path = STPath::new();
    left_path.push_back(STPathElement::inferred(
        account,
        currency_from_string("USD"),
        protocol::AccountID::zero(),
        true,
    ));

    let mut left = STPathSet::new(get_field_by_symbol("sfPaths"));
    left.push_back(left_path.clone());

    let mut right = STPathSet::new(get_field_by_symbol("sfPaths"));
    right.push_back(left_path);
    right.set_fname(get_field_by_symbol("sfAccount"));

    assert!(left.is_equivalent(&right));
    assert_ne!(left, right);
}

#[test]
fn pathset_is_equivalent_returns_false_for_mismatched_stbase_type() {
    let path_set = STPathSet::new(get_field_by_symbol("sfPaths"));
    let blob = STBlob::with_field(get_field_by_symbol("sfPublicKey"));

    assert!(!path_set.is_equivalent(&blob));
}

#[test]
fn pathset_assemble_add_rejects_duplicates_and_keeps_unique_paths() {
    let account =
        parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").expect("genesis account");
    let issuer =
        parse_base58_account_id("rrrrrrrrrrrrrrrrrrrrrhoLvTp").expect("zero account base58");
    let currency = currency_from_string("USD");

    let mut base = STPath::new();
    base.push_back(STPathElement::inferred(account, currency, issuer, true));

    let duplicate_tail = STPathElement::inferred(account, currency, account, true);
    let unique_tail = STPathElement::inferred(protocol::AccountID::zero(), currency, account, true);

    let mut path_set = STPathSet::new(get_field_by_symbol("sfPaths"));
    path_set.push_back({
        let mut existing = base.clone();
        existing.push_back(duplicate_tail.clone());
        existing
    });

    assert!(!path_set.assemble_add(&base, duplicate_tail));
    assert_eq!(path_set.size(), 1);

    assert!(path_set.assemble_add(&base, unique_tail));
    assert_eq!(path_set.size(), 2);
    assert_eq!(path_set[1].size(), 2);
}

#[test]
fn pathset_json_helper_accepts_outer_null_and_hex_fields() {
    let account =
        parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").expect("genesis account");
    let currency = currency_from_string("USD");

    let empty = st_path_set_from_json(get_field_by_symbol("sfPaths"), &JsonValue::Null)
        .expect("null pathset");
    assert!(empty.empty());

    let value = JsonValue::Array(vec![JsonValue::Array(vec![JsonValue::Object(
        [
            (
                "account".to_string(),
                JsonValue::String(str_hex(account.data())),
            ),
            (
                "currency".to_string(),
                JsonValue::String(str_hex(currency.data())),
            ),
            (
                "issuer".to_string(),
                JsonValue::String(str_hex(account.data())),
            ),
        ]
        .into_iter()
        .collect(),
    )])]);

    let parsed =
        st_path_set_from_json(get_field_by_symbol("sfPaths"), &value).expect("hex pathset");
    assert_eq!(parsed.size(), 1);
    assert_eq!(parsed[0][0].account_id(), account);
    assert_eq!(parsed[0][0].currency(), currency);
    assert_eq!(parsed[0][0].issuer_id(), account);
}

#[test]
fn pathset_json_helper_rejects_bad_shapes() {
    let field = get_field_by_symbol("sfPaths");

    assert_eq!(
        st_path_set_from_json(field, &JsonValue::String("bad".to_string())).unwrap_err(),
        "pathset must be an array or null"
    );
    assert_eq!(
        st_path_set_from_json(field, &JsonValue::Array(vec![JsonValue::Bool(true)])).unwrap_err(),
        "path entry must be an array or null"
    );
    assert_eq!(
        st_path_set_from_json(
            field,
            &JsonValue::Array(vec![JsonValue::Array(vec![JsonValue::Bool(true)])]),
        )
        .unwrap_err(),
        "path element must be an object"
    );
    assert_eq!(
        st_path_set_from_json(
            field,
            &JsonValue::Array(vec![JsonValue::Array(vec![JsonValue::Object(
                [("account".to_string(), JsonValue::Bool(true))]
                    .into_iter()
                    .collect(),
            )])]),
        )
        .unwrap_err(),
        "path account must be a string"
    );
    assert_eq!(
        st_path_set_from_json(
            field,
            &JsonValue::Array(vec![JsonValue::Array(vec![JsonValue::Object(
                [("currency".to_string(), JsonValue::Bool(true))]
                    .into_iter()
                    .collect(),
            )])]),
        )
        .unwrap_err(),
        "path currency must be a string"
    );
    assert_eq!(
        st_path_set_from_json(
            field,
            &JsonValue::Array(vec![JsonValue::Array(vec![JsonValue::Object(
                [("issuer".to_string(), JsonValue::Bool(true))]
                    .into_iter()
                    .collect(),
            )])]),
        )
        .unwrap_err(),
        "path issuer must be a string"
    );
}

#[test]
fn pathset_supports_mpt_path_assets_in_wire_and_json_forms() {
    let account =
        parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").expect("genesis account");
    let mpt_id = make_mpt_id(7, account);

    let mut path = STPath::new();
    path.push_back(STPathElement::inferred(
        account,
        PathAsset::from(mpt_id),
        protocol::AccountID::zero(),
        true,
    ));

    let mut path_set = STPathSet::new(get_field_by_symbol("sfPaths"));
    path_set.push_back(path);

    let mut serializer = Serializer::default();
    path_set.add(&mut serializer);

    let mut iter = protocol::SerialIter::new(serializer.data());
    let parsed = STPathSet::from_serial_iter(&mut iter, get_field_by_symbol("sfPaths"));
    assert_eq!(parsed, path_set);
    assert!(parsed[0][0].has_mpt());
    assert_eq!(parsed[0][0].mpt_id(), mpt_id);

    let json_value = JsonValue::Array(vec![JsonValue::Array(vec![JsonValue::Object(
        [
            (
                "account".to_string(),
                JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_string()),
            ),
            ("type".to_string(), JsonValue::Unsigned(65)),
            (
                "mpt_issuance_id".to_string(),
                JsonValue::String(mpt_id.to_string()),
            ),
        ]
        .into_iter()
        .collect(),
    )])]);

    let from_json =
        st_path_set_from_json(get_field_by_symbol("sfPaths"), &json_value).expect("mpt json path");
    assert_eq!(from_json[0][0].mpt_id(), mpt_id);
    assert_eq!(from_json.json(JsonOptions::NONE), json_value);
}
