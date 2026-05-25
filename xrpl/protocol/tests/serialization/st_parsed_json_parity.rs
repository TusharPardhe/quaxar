use std::collections::BTreeMap;

use basics::base_uint::{Uint128, Uint160, Uint192, Uint256};
use protocol::{
    JsonValue, Permission, STParsedJSONObject, StBase, get_field_by_symbol, to_base58, xrp_issue,
};

fn object(entries: impl IntoIterator<Item = (impl Into<String>, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[test]
fn st_parsed_json_scalar_categories_match_current_cpp_shapes() {
    let json = object([
        ("CloseResolution", JsonValue::Bool(true)),
        ("SignerWeight", JsonValue::Unsigned(7)),
        (
            "MaximumAmount",
            JsonValue::String("18446744073709551615".to_owned()),
        ),
        ("LoanScale", JsonValue::Signed(-42)),
    ]);

    let parsed = STParsedJSONObject::new("test", &json)
        .object
        .expect("scalar fields should parse");
    assert_eq!(
        parsed.get_field_u8(get_field_by_symbol("sfCloseResolution")),
        1
    );
    assert_eq!(
        parsed.get_field_u16(get_field_by_symbol("sfSignerWeight")),
        7
    );
    assert_eq!(
        parsed.get_field_u64(get_field_by_symbol("sfMaximumAmount")),
        u64::MAX
    );
    assert_eq!(
        parsed.get_field_i32(get_field_by_symbol("sfLoanScale")),
        -42
    );
}

#[test]
fn st_parsed_json_fixed_width_blob_and_vector_categories_match_cpp_rules() {
    let json = object([
        ("EmailHash", JsonValue::String(String::new())),
        ("TakerPaysCurrency", JsonValue::String(String::new())),
        ("MPTokenIssuanceID", JsonValue::String(String::new())),
        ("LedgerHash", JsonValue::String(String::new())),
        ("SigningPubKey", JsonValue::String(String::new())),
        (
            "Hashes",
            JsonValue::Array(vec![
                JsonValue::String(
                    "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF".to_owned(),
                ),
                JsonValue::String(
                    "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF".to_owned(),
                ),
            ]),
        ),
    ]);

    let parsed = STParsedJSONObject::new("test", &json)
        .object
        .expect("fixed-width and vector fields should parse");
    assert_eq!(
        parsed.get_field_h128(get_field_by_symbol("sfEmailHash")),
        Uint128::zero()
    );
    assert_eq!(
        parsed.get_field_h160(get_field_by_symbol("sfTakerPaysCurrency")),
        Uint160::zero()
    );
    assert_eq!(
        parsed.get_field_h192(get_field_by_symbol("sfMPTokenIssuanceID")),
        Uint192::zero()
    );
    assert_eq!(
        parsed.get_field_h256(get_field_by_symbol("sfLedgerHash")),
        Uint256::zero()
    );
    assert!(
        parsed
            .get_field_vl(get_field_by_symbol("sfSigningPubKey"))
            .is_empty()
    );
    assert_eq!(
        parsed
            .get_field_v256(get_field_by_symbol("sfHashes"))
            .value()
            .len(),
        2
    );
}

#[test]
fn st_parsed_json_account_currency_issue_and_amount_categories_match_cpp_rules() {
    let json = object([
        (
            "Account",
            JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
        ),
        ("BaseAsset", JsonValue::String("usd".to_owned())),
        (
            "Asset",
            object([
                ("currency", JsonValue::String("USD".to_owned())),
                (
                    "issuer",
                    JsonValue::String("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe".to_owned()),
                ),
            ]),
        ),
        ("Fee", JsonValue::String("10".to_owned())),
        (
            "Amount",
            object([
                ("value", JsonValue::String("12.5".to_owned())),
                ("currency", JsonValue::String("USD".to_owned())),
                (
                    "issuer",
                    JsonValue::String("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe".to_owned()),
                ),
            ]),
        ),
    ]);

    let parsed = STParsedJSONObject::new("test", &json)
        .object
        .expect("account/currency/issue/amount fields should parse");
    assert_eq!(
        to_base58(parsed.get_account_id(get_field_by_symbol("sfAccount"))),
        "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh"
    );
    assert_eq!(
        parsed
            .get_field_currency(get_field_by_symbol("sfBaseAsset"))
            .text(),
        "usd"
    );
    assert_eq!(
        parsed
            .get_field_issue(get_field_by_symbol("sfAsset"))
            .asset(),
        protocol::Asset::Issue(protocol::Issue::new(
            protocol::currency_from_string("USD"),
            protocol::parse_base58_account_id("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe")
                .expect("issuer account"),
        ))
    );
    assert_eq!(
        parsed
            .get_field_amount(get_field_by_symbol("sfFee"))
            .asset(),
        protocol::Asset::Issue(xrp_issue())
    );
    assert_eq!(
        parsed
            .get_field_amount(get_field_by_symbol("sfAmount"))
            .asset(),
        protocol::Asset::Issue(protocol::Issue::new(
            protocol::currency_from_string("USD"),
            protocol::parse_base58_account_id("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe")
                .expect("issuer account"),
        ))
    );
}

#[test]
fn st_parsed_json_pathset_bridge_and_number_categories_match_cpp_rules() {
    let json = object([
        (
            "Paths",
            JsonValue::Array(vec![JsonValue::Array(vec![object([
                (
                    "account",
                    JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
                ),
                ("currency", JsonValue::String("USD".to_owned())),
                (
                    "issuer",
                    JsonValue::String("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe".to_owned()),
                ),
            ])])]),
        ),
        (
            "XChainBridge",
            object([
                (
                    "LockingChainDoor",
                    JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
                ),
                (
                    "LockingChainIssue",
                    object([("currency", JsonValue::String("XRP".to_owned()))]),
                ),
                (
                    "IssuingChainDoor",
                    JsonValue::String("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe".to_owned()),
                ),
                (
                    "IssuingChainIssue",
                    object([
                        ("currency", JsonValue::String("USD".to_owned())),
                        (
                            "issuer",
                            JsonValue::String("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe".to_owned()),
                        ),
                    ]),
                ),
            ]),
        ),
        ("AssetsAvailable", JsonValue::String("12.5".to_owned())),
    ]);

    let parsed = STParsedJSONObject::new("test", &json)
        .object
        .expect("pathset, bridge, and number fields should parse");
    let paths = parsed.get_field_path_set(get_field_by_symbol("sfPaths"));
    assert_eq!(paths[0].size(), 1);
    assert!(paths[0][0].has_currency());
    assert_eq!(
        to_base58(paths[0][0].account_id()),
        "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh"
    );

    let bridge = parsed.get_field_xchain_bridge(get_field_by_symbol("sfXChainBridge"));
    assert_eq!(
        to_base58(bridge.locking_chain_door()),
        "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh"
    );
    assert_eq!(
        bridge.issuing_chain_issue(),
        protocol::Asset::Issue(protocol::Issue::new(
            protocol::currency_from_string("USD"),
            protocol::parse_base58_account_id("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe")
                .expect("issuer account"),
        ))
    );
    assert_eq!(
        parsed
            .get_field_number(get_field_by_symbol("sfAssetsAvailable"))
            .value()
            .to_string(),
        "12.5"
    );
}

#[test]
fn st_parsed_json_object_array_and_permission_categories_match_cpp_rules() {
    let json = object([
        (
            "TransactionMetaData",
            object([("TransactionResult", JsonValue::Unsigned(1))]),
        ),
        (
            "SignerEntries",
            JsonValue::Array(vec![object([(
                "TransactionMetaData",
                object([("TransactionResult", JsonValue::Unsigned(2))]),
            )])]),
        ),
        (
            "Permissions",
            JsonValue::Array(vec![object([(
                "Permission",
                object([(
                    "PermissionValue",
                    JsonValue::String("PaymentMint".to_owned()),
                )]),
            )])]),
        ),
    ]);

    let parsed = STParsedJSONObject::new("test", &json)
        .object
        .expect("object and array fields should parse");
    assert_eq!(
        parsed
            .get_field_object(get_field_by_symbol("sfTransactionMetaData"))
            .get_field_u8(get_field_by_symbol("sfTransactionResult")),
        1
    );
    let signer_entries = parsed.get_field_array(get_field_by_symbol("sfSignerEntries"));
    let first = signer_entries.get(0).expect("single array element");
    assert_eq!(first.fname(), get_field_by_symbol("sfTransactionMetaData"));
    assert_eq!(
        first.get_field_u8(get_field_by_symbol("sfTransactionResult")),
        2
    );

    let permissions = parsed.get_field_array(get_field_by_symbol("sfPermissions"));
    assert_eq!(
        permissions
            .get(0)
            .expect("permission object")
            .get_field_u32(get_field_by_symbol("sfPermissionValue")),
        Permission::get_instance()
            .get_granular_value("PaymentMint")
            .expect("permission value")
    );
}

#[test]
fn st_parsed_json_edge_case_failures_match_current_cpp_rules() {
    let unknown = object([("NotARealField", JsonValue::Unsigned(1))]);
    assert!(STParsedJSONObject::new("test", &unknown).object.is_none());

    let invalid_array = object([(
        "SignerEntries",
        JsonValue::Array(vec![object([
            ("TransactionResult", JsonValue::Unsigned(2)),
            ("NetworkID", JsonValue::Unsigned(3)),
        ])]),
    )]);
    assert!(
        STParsedJSONObject::new("test", &invalid_array)
            .object
            .is_none()
    );

    let invalid_blob = object([("SigningPubKey", JsonValue::String("nothex".to_owned()))]);
    assert!(
        STParsedJSONObject::new("test", &invalid_blob)
            .object
            .is_none()
    );

    let invalid_bridge = object([(
        "XChainBridge",
        object([
            (
                "LockingChainDoor",
                JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
            ),
            ("ExtraField", JsonValue::Unsigned(1)),
        ]),
    )]);
    assert!(
        STParsedJSONObject::new("test", &invalid_bridge)
            .object
            .is_none()
    );
}
