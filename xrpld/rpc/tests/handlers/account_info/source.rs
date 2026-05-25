//! Account info source tests.

use super::*;

#[test]
fn application_account_info_source_reads_live_ledgers_features_and_queue_shell() {
    let account = sample_account(0x80);
    let signer = sample_account(0x81);
    let now_close_time = current_net_close_time();

    let current_ledger = ledger_with_state_entries(
        200,
        now_close_time.saturating_add(1),
        [
            make_account_root(
                account,
                lsfAllowTrustLineClawback | lsfAllowTrustLineLocking,
                None,
            ),
            make_signer_list(account, signer),
        ],
        Rules::new([feature_clawback(), feature_token_escrow()]),
    );
    let closed_ledger = ledger_with_state_entries(
        199,
        now_close_time,
        [make_account_root(account, 0, None)],
        Rules::default(),
    );
    let validated_ledger = ledger_with_state_entries(
        198,
        now_close_time.saturating_sub(1),
        [make_account_root(account, 0, None)],
        Rules::default(),
    );

    let app = ApplicationRoot::new(0).expect("root shell should build");
    app.on_closed_ledger(closed_ledger);
    let _ = app.on_validated_ledger(validated_ledger);

    let _unused_boundary_source = ApplicationAccountInfoSource::new(&app);
    let source = ApplicationAccountInfoSource::with_current_ledger(&app, current_ledger);
    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("signer_lists", JsonValue::Bool(true)),
                ("queue", JsonValue::Bool(true)),
            ]),
            api_version: 2,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        result.get("ledger_current_index"),
        Some(&JsonValue::Unsigned(200))
    );
    assert_eq!(result.get("validated"), Some(&JsonValue::Bool(false)));
    assert!(matches!(
        result.get("signer_lists"),
        Some(JsonValue::Array(values)) if values.len() == 1
    ));
    assert_eq!(
        result.get("queue_data"),
        Some(&JsonValue::Object(std::collections::BTreeMap::from([(
            "txn_count".to_owned(),
            JsonValue::Unsigned(0),
        )])))
    );

    let JsonValue::Object(account_flags) = result
        .get("account_flags")
        .expect("account_flags should exist")
    else {
        panic!("account_flags must be an object");
    };
    assert_eq!(
        account_flags.get("allowTrustLineClawback"),
        Some(&JsonValue::Bool(true))
    );
    assert_eq!(
        account_flags.get("allowTrustLineLocking"),
        Some(&JsonValue::Bool(true))
    );
}

#[test]
fn account_info_invalid_account_types_all_variants() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    // Float-like (Signed)
    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([("account", JsonValue::Signed(11))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );
    assert_eq!(
        result.get("error_message"),
        Some(&JsonValue::String("Invalid field 'account'.".to_owned()))
    );

    // Boolean
    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([("account", JsonValue::Bool(true))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );

    // Null
    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([("account", JsonValue::Null)]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );

    // Object
    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([("account", JsonValue::Object(Default::default()))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );

    // Array
    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([("account", JsonValue::Array(vec![]))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );
}

#[test]
fn account_info_malformed_account_string_returns_error_code() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    // Node public key format (not an account)
    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([(
                "account",
                JsonValue::String(
                    "n94JNrQYkDrpt62bbSR7nVEhdyAvcJXRAsjEkFYyqRkh9SUTYEqV".to_owned(),
                ),
            )]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("actMalformed".to_owned()))
    );
    assert_eq!(result.get("error_code"), Some(&JsonValue::Signed(35)));
    assert_eq!(
        result.get("error_message"),
        Some(&JsonValue::String("Account malformed.".to_owned()))
    );
}

#[test]
fn account_info_account_data_contains_expected_fields() {
    let ledger = closed_ledger();
    let account = sample_account(0x30);

    let mut source = FakeSource {
        ledger: Some(ledger),
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account, 0, None));

    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None);

    // Verify ledger info
    assert_eq!(
        result.get("ledger_hash"),
        Some(&JsonValue::String(ledger.hash.to_string()))
    );
    assert_eq!(
        result.get("ledger_index"),
        Some(&JsonValue::Unsigned(u64::from(ledger.seq)))
    );
    assert_eq!(result.get("validated"), Some(&JsonValue::Bool(true)));

    // Verify account_data exists and has expected fields
    let JsonValue::Object(account_data) = result.get("account_data").expect("account_data") else {
        panic!("account_data must be an object");
    };
    assert_eq!(
        account_data.get("Account"),
        Some(&JsonValue::String(to_base58(account)))
    );
    assert_eq!(account_data.get("Sequence"), Some(&JsonValue::Unsigned(7)));
    assert_eq!(
        account_data.get("OwnerCount"),
        Some(&JsonValue::Unsigned(2))
    );
    assert!(account_data.contains_key("Balance"));
    assert!(account_data.contains_key("Flags"));
}

#[test]
fn account_info_without_signer_lists_omits_them() {
    let ledger = closed_ledger();
    let account = sample_account(0x31);
    let signer = sample_account(0x32);

    let mut source = FakeSource {
        ledger: Some(ledger),
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account, 0, None));
    source
        .signer_lists
        .insert(account, make_signer_list(account, signer));

    // Without signer_lists param
    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(account_data) = result.get("account_data").expect("account_data") else {
        panic!("account_data must be an object");
    };
    assert!(
        !account_data.contains_key("signer_lists"),
        "signer_lists should not be present without param"
    );
}

#[test]
fn account_info_signer_list_details_v1() {
    let ledger = closed_ledger();
    let account = sample_account(0x33);
    let signer = sample_account(0x34);

    let mut source = FakeSource {
        ledger: Some(ledger),
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account, 0, None));
    source
        .signer_lists
        .insert(account, make_signer_list(account, signer));

    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("signer_lists", JsonValue::Bool(true)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(account_data) = result.get("account_data").expect("account_data") else {
        panic!("account_data must be an object");
    };
    // In v1, signer_lists is inside account_data
    let JsonValue::Array(signer_lists) = account_data.get("signer_lists").expect("signer_lists")
    else {
        panic!("signer_lists must be an array");
    };
    assert_eq!(signer_lists.len(), 1);

    let JsonValue::Object(signer_list) = &signer_lists[0] else {
        panic!("signer_list must be an object");
    };
    assert_eq!(
        signer_list.get("SignerQuorum"),
        Some(&JsonValue::Unsigned(2))
    );
    let JsonValue::Array(entries) = signer_list.get("SignerEntries").expect("SignerEntries") else {
        panic!("SignerEntries must be an array");
    };
    assert_eq!(entries.len(), 1);
    let JsonValue::Object(entry) = &entries[0] else {
        panic!("entry must be an object");
    };
    // The entry should have SignerEntry wrapper or direct fields
    let entry_inner = entry
        .get("SignerEntry")
        .map(|v| {
            let JsonValue::Object(o) = v else {
                panic!("SignerEntry must be an object");
            };
            o
        })
        .unwrap_or(entry);
    assert_eq!(
        entry_inner.get("Account"),
        Some(&JsonValue::String(to_base58(signer)))
    );
    assert_eq!(
        entry_inner.get("SignerWeight"),
        Some(&JsonValue::Unsigned(3))
    );
}

#[test]
fn account_info_queue_not_returned_for_closed_ledger() {
    let ledger = closed_ledger();
    let account = sample_account(0x35);

    let mut source = FakeSource {
        ledger: Some(ledger),
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account, 0, None));

    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("queue", JsonValue::Bool(true)),
            ]),
            api_version: 2,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    // Queue data should not be present for closed/validated ledger
    assert!(
        !result.contains_key("queue_data"),
        "queue_data should not be present for closed ledger"
    );
}

#[test]
fn account_info_disallow_incoming_flags() {
    use protocol::lsfDisallowIncomingNFTokenOffer;

    let ledger = closed_ledger();
    let account = sample_account(0x36);

    let mut source = FakeSource {
        ledger: Some(ledger),
        ..Default::default()
    };
    source.account_roots.insert(
        account,
        make_account_root(
            account,
            lsfDisallowIncomingCheck
                | lsfDisallowIncomingTrustline
                | lsfDisallowIncomingNFTokenOffer,
            None,
        ),
    );

    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 2,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(account_flags) = result.get("account_flags").expect("account_flags")
    else {
        panic!("account_flags must be an object");
    };
    assert_eq!(
        account_flags.get("disallowIncomingCheck"),
        Some(&JsonValue::Bool(true))
    );
    assert_eq!(
        account_flags.get("disallowIncomingTrustline"),
        Some(&JsonValue::Bool(true))
    );
    assert_eq!(
        account_flags.get("disallowIncomingNFTokenOffer"),
        Some(&JsonValue::Bool(true))
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// ═══════════════════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════════════════
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn cpp_account_info_empty_params_error() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    let params = JsonValue::Object(Default::default());
    let result = do_account_info(
        &AccountInfoRequest {
            params: &params,
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    assert!(result.get("error").is_some() || result.get("error_message").is_some());
}

#[test]
fn cpp_account_info_not_found_error() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    let mut params_map = std::collections::BTreeMap::new();
    params_map.insert(
        "account".to_owned(),
        JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
    );
    let params = JsonValue::Object(params_map);
    let result = do_account_info(
        &AccountInfoRequest {
            params: &params,
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    assert!(result.get("error").is_some());
}

#[test]
fn cpp_account_info_malformed_account() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    let mut params_map = std::collections::BTreeMap::new();
    params_map.insert(
        "account".to_owned(),
        JsonValue::String("not_a_valid_account".to_owned()),
    );
    let params = JsonValue::Object(params_map);
    let result = do_account_info(
        &AccountInfoRequest {
            params: &params,
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    assert!(result.get("error").is_some());
}
