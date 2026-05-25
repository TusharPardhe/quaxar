//! ledger entry helpers tests part A.

use super::*;

#[test]
fn error_helpers_match_cpp_json_shape() {
    assert_eq!(
        error_fields(&missing_field_error("account")),
        ("malformedRequest", 31, "Missing field 'account'.")
    );
    assert_eq!(
        error_fields(&expected_field_error(
            "malformedAddress",
            "account",
            "AccountID"
        )),
        (
            "malformedAddress",
            31,
            "Invalid field 'account', not AccountID."
        )
    );
    assert_eq!(
        error_fields(&malformed_error("badSyntax", "custom message")),
        ("badSyntax", 31, "custom message")
    );
}

#[test]
fn account_and_numeric_parsers_match_current_cpp_behaviour() {
    let genesis = to_base58(xrp_account());
    let account = parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh")
        .expect("genesis should parse");

    assert_eq!(parse_account_id(&JsonValue::String(genesis)), None);
    assert_eq!(
        parse_account_id(&JsonValue::String(
            "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()
        )),
        Some(account)
    );
    assert_eq!(
        required_account_id(&object([]), "account", "malformedRequest"),
        Err(missing_field_error("account"))
    );
    assert_eq!(parse_uint32(&JsonValue::Unsigned(9)), Some(9));
    assert_eq!(parse_uint32(&JsonValue::String("9".to_owned())), Some(9));
    assert_eq!(parse_uint32(&JsonValue::Bool(true)), None);
    assert_eq!(
        parse_uint256(&JsonValue::String("00".repeat(32))),
        Some(Uint256::zero())
    );
    assert_eq!(
        parse_uint192(&JsonValue::String("00".repeat(24))),
        Some(protocol::MPTID::zero())
    );
    assert_eq!(
        required_uint32(
            &object([("seq", JsonValue::Unsigned(7))]),
            "seq",
            "malformedSeq"
        ),
        Ok(7)
    );
    assert_eq!(
        required_uint256(
            &object([("id", JsonValue::String("00".repeat(32)))]),
            "id",
            "malformedId"
        ),
        Ok(Uint256::zero())
    );
    assert_eq!(
        required_uint192(
            &object([("id", JsonValue::String("00".repeat(24)))]),
            "id",
            "malformedId"
        ),
        Ok(protocol::MPTID::zero())
    );
}

#[test]
fn issue_asset_and_blob_helpers_match_cpp_validation_rules() {
    let issue_json = object([
        ("currency", JsonValue::String("USD".to_owned())),
        (
            "issuer",
            JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
        ),
    ]);
    let asset_json = object([("mpt_issuance_id", JsonValue::String("00".repeat(24)))]);

    assert!(parse_issue(&issue_json).is_some());
    assert!(parse_asset(&asset_json).is_some());
    assert_eq!(
        required_issue(
            &object([("issue", issue_json.clone())]),
            "issue",
            "malformedIssue"
        )
        .unwrap()
        .currency,
        protocol::currency_from_string("USD")
    );
    assert!(
        required_asset(
            &object([("asset", asset_json.clone())]),
            "asset",
            "malformedAsset"
        )
        .is_ok()
    );
    assert_eq!(
        parse_hex_blob(&JsonValue::String("0A0B".to_owned()), 8),
        Some(vec![0x0a, 0x0b])
    );
    assert_eq!(parse_hex_blob(&JsonValue::String("".to_owned()), 8), None);
    assert_eq!(
        required_hex_blob(
            &object([("blob", JsonValue::String("0A0B".to_owned()))]),
            "blob",
            8,
            "malformedBlob"
        ),
        Ok(vec![0x0a, 0x0b])
    );
}

#[test]
fn selector_helpers_cover_common_ledger_entry_paths() {
    let owner = AccountID::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
        .expect("owner should parse");
    let other = AccountID::from_hex("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB")
        .expect("other should parse");
    let owner_json = JsonValue::String(to_base58(owner));
    let other_json = JsonValue::String(to_base58(other));

    assert_eq!(
        parse_account_root(&object([("account", owner_json.clone())]), "account", 2),
        Ok(account_keylet(Uint160::from_slice(owner.data()).expect("owner width")).key)
    );
    assert_eq!(
        parse_directory_node(
            &object([
                ("owner", owner_json.clone()),
                ("sub_index", JsonValue::Unsigned(3)),
            ]),
            "dir",
            2,
        ),
        Ok(protocol::page_keylet(
            owner_dir_keylet(Uint160::from_slice(owner.data()).expect("owner width")),
            3
        )
        .key)
    );
    assert_eq!(
        parse_credential(
            &object([
                ("subject", owner_json.clone()),
                ("issuer", other_json.clone()),
                (
                    "credential_type",
                    JsonValue::String("6162636465".to_owned())
                ),
            ]),
            "credential",
        ),
        Ok(credential_keylet(
            Uint160::from_slice(owner.data()).expect("owner width"),
            Uint160::from_slice(other.data()).expect("other width"),
            b"abcde",
        )
        .key)
    );
    assert_eq!(
        parse_deposit_preauth_account(
            &object([
                ("owner", owner_json.clone()),
                ("authorized", other_json.clone()),
            ]),
            "deposit_preauth",
        ),
        Ok(deposit_preauth_keylet(
            Uint160::from_slice(owner.data()).expect("owner width"),
            Uint160::from_slice(other.data()).expect("other width"),
        )
        .key)
    );
    let credential_hash = sha512_half(&[other.data(), b"abcde"]);
    assert_eq!(
        parse_deposit_preauth_account(
            &object([
                ("owner", owner_json.clone()),
                (
                    "authorized_credentials",
                    JsonValue::Array(vec![object([
                        ("issuer", other_json.clone()),
                        (
                            "credential_type",
                            JsonValue::String("6162636465".to_owned())
                        ),
                    ])]),
                ),
            ]),
            "deposit_preauth",
        ),
        Ok(deposit_preauth_credentials_keylet(
            Uint160::from_slice(owner.data()).expect("owner width"),
            &[credential_hash],
        )
        .key)
    );
    assert_eq!(
        parse_ripple_state(
            &object([
                ("currency", JsonValue::String("USD".to_owned()),),
                (
                    "accounts",
                    JsonValue::Array(vec![owner_json.clone(), other_json.clone()]),
                ),
            ]),
            "state",
        ),
        Ok(line(owner, other, protocol::currency_from_string("USD"),).key)
    );
    let issuance_id = protocol::MPTID::from_hex("000102030405060708090A0B0C0D0E0F1011121314151617")
        .expect("issuance id should parse");
    let issuance_key = protocol::mpt_issuance_keylet_from_mptid(issuance_id);
    assert_eq!(
        parse_mpt_issuance(
            &JsonValue::String("000102030405060708090A0B0C0D0E0F1011121314151617".to_owned()),
            "mpt_issuance",
        ),
        Ok(issuance_key.key)
    );
    assert_eq!(
        parse_mptoken(
            &object([
                (
                    "mpt_issuance_id",
                    JsonValue::String(
                        "000102030405060708090A0B0C0D0E0F1011121314151617".to_owned()
                    ),
                ),
                ("account", JsonValue::String(to_base58(owner))),
            ]),
            "mptoken",
        ),
        Ok(protocol::mptoken_keylet_from_mptid(
            issuance_id,
            Uint160::from_slice(owner.data()).expect("owner width"),
        )
        .key)
    );
}

#[test]
fn bridge_and_xchain_selector_helpers_match_current_cpp_rules() {
    let locking_door = AccountID::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
        .expect("locking door should parse");
    let issuing_door = AccountID::from_hex("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB")
        .expect("issuing door should parse");
    let bridge_json = object([
        (
            "LockingChainDoor",
            JsonValue::String(to_base58(locking_door)),
        ),
        (
            "LockingChainIssue",
            object([
                ("currency", JsonValue::String("USD".to_owned())),
                (
                    "issuer",
                    JsonValue::String("rrrrrrrrrrrrrrrrrrrrrhoLvTp".to_owned()),
                ),
            ]),
        ),
        (
            "IssuingChainDoor",
            JsonValue::String(to_base58(issuing_door)),
        ),
        (
            "IssuingChainIssue",
            object([
                ("currency", JsonValue::String("CNY".to_owned())),
                (
                    "issuer",
                    JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
                ),
            ]),
        ),
    ]);
    let parsed_bridge = parse_bridge_fields(&bridge_json).expect("bridge fields should parse");

    assert_eq!(
        parse_bridge(
            &object([
                ("bridge", bridge_json.clone()),
                ("bridge_account", JsonValue::String(to_base58(locking_door)),),
            ]),
            "bridge",
        ),
        Ok(bridge_keylet_from_door_issue(
            Uint160::from_slice(locking_door.data()).expect("door width"),
            parsed_bridge.locking_chain_issue,
        )
        .key)
    );
    assert_eq!(
        parse_xchain_owned_claim_id(
            &object([
                (
                    "LockingChainDoor",
                    JsonValue::String(to_base58(locking_door))
                ),
                (
                    "LockingChainIssue",
                    object([
                        ("currency", JsonValue::String("USD".to_owned())),
                        (
                            "issuer",
                            JsonValue::String("rrrrrrrrrrrrrrrrrrrrrhoLvTp".to_owned()),
                        ),
                    ]),
                ),
                (
                    "IssuingChainDoor",
                    JsonValue::String(to_base58(issuing_door))
                ),
                (
                    "IssuingChainIssue",
                    object([
                        ("currency", JsonValue::String("CNY".to_owned())),
                        (
                            "issuer",
                            JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
                        ),
                    ]),
                ),
                ("xchain_owned_claim_id", JsonValue::Unsigned(7)),
            ]),
            "xchain_owned_claim_id",
        ),
        Ok(xchain_owned_claim_id_keylet_from_bridge(
            Uint160::from_slice(locking_door.data()).expect("door width"),
            parsed_bridge.locking_chain_issue,
            Uint160::from_slice(issuing_door.data()).expect("door width"),
            parsed_bridge.issuing_chain_issue,
            7,
        )
        .key)
    );
    assert_eq!(
        parse_xchain_owned_create_account_claim_id(
            &object([
                (
                    "LockingChainDoor",
                    JsonValue::String(to_base58(locking_door))
                ),
                (
                    "LockingChainIssue",
                    object([
                        ("currency", JsonValue::String("USD".to_owned())),
                        (
                            "issuer",
                            JsonValue::String("rrrrrrrrrrrrrrrrrrrrrhoLvTp".to_owned()),
                        ),
                    ]),
                ),
                (
                    "IssuingChainDoor",
                    JsonValue::String(to_base58(issuing_door))
                ),
                (
                    "IssuingChainIssue",
                    object([
                        ("currency", JsonValue::String("CNY".to_owned())),
                        (
                            "issuer",
                            JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
                        ),
                    ]),
                ),
                (
                    "xchain_owned_create_account_claim_id",
                    JsonValue::Unsigned(7),
                ),
            ]),
            "xchain_owned_create_account_claim_id",
        ),
        Ok(xchain_owned_create_account_claim_id_keylet_from_bridge(
            Uint160::from_slice(locking_door.data()).expect("door width"),
            parsed_bridge.locking_chain_issue,
            Uint160::from_slice(issuing_door.data()).expect("door width"),
            parsed_bridge.issuing_chain_issue,
            7,
        )
        .key)
    );
}

#[test]
fn index_helpers_match_special_cases_and_numeric_skip_lists() {
    assert_eq!(
        parse_index(&JsonValue::String("amendments".to_owned()), "index", 3),
        Ok(protocol::amendments_key())
    );
    assert_eq!(
        parse_index(&JsonValue::Unsigned(123), "index", 2),
        Ok(protocol::skip_keylet_for_ledger(123).key)
    );
    assert_eq!(
        parse_ledger_hashes(&JsonValue::Bool(true), "ledger_hashes", 2),
        Ok(protocol::skip_keylet().key)
    );
    assert_eq!(
        parse_ledger_hashes(&JsonValue::Unsigned(9), "ledger_hashes", 2),
        Ok(protocol::skip_keylet_for_ledger(9).key)
    );
}

#[test]
fn mpt_selector_helpers_match_current_cpp_rules() {
    let holder = AccountID::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
        .expect("holder should parse");
    let issuance_id = protocol::MPTID::from_hex("000102030405060708090A0B0C0D0E0F1011121314151617")
        .expect("issuance id should parse");
    let issuance_key = sha512_half(&[&u16::from(b'~').to_be_bytes(), issuance_id.data()]);
    let token_key = sha512_half(&[
        &u16::from(b't').to_be_bytes(),
        issuance_key.data(),
        holder.data(),
    ]);

    assert_eq!(
        parse_mpt_issuance(
            &JsonValue::String("000102030405060708090A0B0C0D0E0F1011121314151617".to_owned()),
            "mpt_issuance",
        ),
        Ok(issuance_key)
    );
    assert_eq!(
        parse_mptoken(
            &object([
                (
                    "mpt_issuance_id",
                    JsonValue::String(
                        "000102030405060708090A0B0C0D0E0F1011121314151617".to_owned()
                    ),
                ),
                ("account", JsonValue::String(to_base58(holder))),
            ]),
            "mptoken",
        ),
        Ok(token_key)
    );
    assert_eq!(
        parse_mptoken(
            &JsonValue::String(
                "00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF".to_owned()
            ),
            "mptoken",
        ),
        Ok(
            Uint256::from_hex("00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF")
                .expect("object id should parse")
        )
    );

    let error = parse_mpt_issuance(&JsonValue::String("00".repeat(23)), "mpt_issuance")
        .expect_err("short issuance ids should fail");
    assert_eq!(
        error_fields(&error),
        (
            "malformedMPTokenIssuance",
            31,
            "Invalid field 'mpt_issuance', not Hash192."
        )
    );
}

#[test]
fn deposit_preauth_credential_array_error_messages() {
    let empty = parse_deposit_preauth_credential_array(
        &JsonValue::Array(Vec::new()),
        "authorized_credentials",
    )
    .unwrap_err();
    assert_eq!(
        error_fields(&empty),
        (
            "malformedAuthorizedCredentials",
            31,
            "Invalid field 'authorized_credentials', array empty."
        )
    );

    let too_long = parse_deposit_preauth_credential_array(
        &JsonValue::Array(
            (0..=MAX_CREDENTIALS_ARRAY_SIZE)
                .map(|idx| {
                    object([
                        (
                            "issuer",
                            JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
                        ),
                        ("credential_type", JsonValue::String(format!("{idx:02X}"))),
                    ])
                })
                .collect(),
        ),
        "authorized_credentials",
    )
    .unwrap_err();
    assert_eq!(
        error_fields(&too_long),
        (
            "malformedAuthorizedCredentials",
            31,
            "Invalid field 'authorized_credentials', array too long."
        )
    );
}

#[test]
fn deposit_preauth_credential_duplicates_match_cpp_error_shape() {
    let duplicated = parse_deposit_preauth_account(
        &object([
            (
                "owner",
                JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
            ),
            (
                "authorized_credentials",
                JsonValue::Array(vec![
                    object([
                        (
                            "issuer",
                            JsonValue::String("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe".to_owned()),
                        ),
                        (
                            "credential_type",
                            JsonValue::String("6162636465".to_owned()),
                        ),
                    ]),
                    object([
                        (
                            "issuer",
                            JsonValue::String("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe".to_owned()),
                        ),
                        (
                            "credential_type",
                            JsonValue::String("6162636465".to_owned()),
                        ),
                    ]),
                ]),
            ),
        ]),
        "deposit_preauth",
    )
    .unwrap_err();
    assert_eq!(
        error_fields(&duplicated),
        (
            "malformedAuthorizedCredentials",
            31,
            "Invalid field 'authorized_credentials', not array."
        )
    );
}
