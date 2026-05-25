//! AMM info handler tests part 1.

use super::*;

#[test]
fn amm_info_rejects_missing_assets_when_api_is_old() {
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        validated: Some(closed_ledger()),
        ..Default::default()
    };
    let request = request(
        object([(
            "asset",
            issue_json(Issue::new(
                currency_from_string("USD"),
                sample_account(0x21),
            )),
        )]),
        2,
    );

    let result = do_amm_info(&request, &source);
    assert_eq!(
        error_fields(&result),
        ("invalidParams", 31, "Invalid parameters.")
    );

    // Keep the source live so the compiler sees the borrowed trait object path.
    source.ledger = None;
}

#[test]
fn amm_info_prefers_issue_and_account_parsing_over_late_invalid_params_on_newer_api() {
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        validated: Some(closed_ledger()),
        ..Default::default()
    };
    let amm_account = sample_account(0x40);
    source
        .account_roots
        .insert(amm_account, make_account_root(amm_account, None, false));

    let request = request(
        object([
            (
                "asset",
                issue_json(Issue::new(
                    currency_from_string("USD"),
                    sample_account(0x11),
                )),
            ),
            (
                "amm_account",
                JsonValue::String(to_base58(sample_account(0x22))),
            ),
        ]),
        3,
    );

    let result = do_amm_info(&request, &source);
    assert_eq!(
        error_fields(&result),
        ("actMalformed", 35, "Account malformed.")
    );
}

#[test]
fn amm_info_reports_malformed_issue_json() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        validated: Some(closed_ledger()),
        ..Default::default()
    };
    let request = request(
        object([
            ("asset", JsonValue::String("not-an-issue".to_owned())),
            (
                "asset2",
                issue_json(Issue::new(
                    currency_from_string("EUR"),
                    sample_account(0x20),
                )),
            ),
        ]),
        3,
    );

    let result = do_amm_info(&request, &source);
    assert_eq!(
        error_fields(&result),
        ("issueMalformed", 93, "Issue is malformed.")
    );
}

#[test]
fn amm_info_returns_supported_shape_for_limited_slice() {
    let amm_account = sample_account(0x40);
    let issuer1 = sample_account(0x60);
    let issuer2 = sample_account(0x20);
    let query_account = sample_account(0x41);
    let issue1 = Issue::new(currency_from_string("USD"), issuer1);
    let issue2 = Issue::new(currency_from_string("EUR"), issuer2);
    let amm_key = amm(Asset::from(issue1), Asset::from(issue2)).key;

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        validated: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .account_roots
        .insert(issuer1, make_account_root(issuer1, None, true));
    source
        .account_roots
        .insert(issuer2, make_account_root(issuer2, None, false));
    source.account_roots.insert(
        amm_account,
        make_account_root(amm_account, Some(amm_key), false),
    );
    source
        .account_roots
        .insert(query_account, make_account_root(query_account, None, false));
    source.entries.insert(
        amm_key,
        make_amm_entry(amm_account, amm_key, issue1, issue2),
    );
    source.entries.insert(
        line(amm_account, issuer2, issue2.currency).key,
        make_trust_line(amm_account, issuer2, issue2.currency, lsfLowFreeze),
    );

    let request = request(
        object([
            ("asset", issue_json(issue1)),
            ("asset2", issue_json(issue2)),
            ("account", JsonValue::String(to_base58(query_account))),
        ]),
        3,
    );

    let result = do_amm_info(&request, &source);
    let object = json_object(&result);
    assert_eq!(object.get("ledger_index"), Some(&JsonValue::Unsigned(404)));
    assert_eq!(object.get("validated"), Some(&JsonValue::Bool(true)));

    let amm = json_object(object.get("amm").expect("amm result"));
    assert_eq!(
        json_string(amm.get("account").expect("account")),
        to_base58(amm_account)
    );
    assert_eq!(amm.get("trading_fee"), Some(&JsonValue::Unsigned(17)));
    let lp_token = json_object(amm.get("lp_token").expect("lp_token"));
    assert_eq!(
        lp_token.get("value"),
        Some(&JsonValue::String("5600".to_owned()))
    );
    assert_eq!(
        lp_token.get("issuer"),
        Some(&JsonValue::String(to_base58(amm_account)))
    );
    assert_eq!(
        lp_token.get("currency"),
        Some(&JsonValue::String(protocol::currency_to_string(
            currency_from_string("LPT")
        )))
    );

    let vote_slots = match amm.get("vote_slots") {
        Some(JsonValue::Array(votes)) => votes,
        other => panic!("expected vote slots, got {other:?}"),
    };
    assert_eq!(vote_slots.len(), 1);
    let vote = json_object(&vote_slots[0]);
    assert_eq!(
        vote.get("account"),
        Some(&JsonValue::String(to_base58(sample_account(0x70))))
    );
    assert_eq!(vote.get("trading_fee"), Some(&JsonValue::Unsigned(25)));
    assert_eq!(vote.get("vote_weight"), Some(&JsonValue::Unsigned(12_500)));

    let auction = json_object(amm.get("auction_slot").expect("auction_slot"));
    assert_eq!(
        auction.get("account"),
        Some(&JsonValue::String(to_base58(sample_account(0x71))))
    );
    assert_eq!(
        auction.get("discounted_fee"),
        Some(&JsonValue::Unsigned(17))
    );
    assert_eq!(
        auction.get("expiration"),
        Some(&JsonValue::Unsigned(123_456))
    );
    let price = json_object(auction.get("price").expect("price"));
    assert_eq!(
        price.get("value"),
        Some(&JsonValue::String("5600".to_owned()))
    );
    assert_eq!(
        price.get("currency"),
        Some(&JsonValue::String(protocol::currency_to_string(
            currency_from_string("LPT")
        )))
    );
    assert_eq!(
        price.get("issuer"),
        Some(&JsonValue::String(to_base58(amm_account)))
    );

    let auth_accounts = match auction.get("auth_accounts") {
        Some(JsonValue::Array(entries)) => entries,
        other => panic!("expected auth accounts, got {other:?}"),
    };
    assert_eq!(auth_accounts.len(), 2);
    assert_eq!(
        json_object(&auth_accounts[0]).get("account"),
        Some(&JsonValue::String(to_base58(sample_account(0x72))))
    );
    assert_eq!(
        json_object(&auth_accounts[1]).get("account"),
        Some(&JsonValue::String(to_base58(sample_account(0x73))))
    );

    assert_eq!(amm.get("asset_frozen"), Some(&JsonValue::Bool(true)));
    assert_eq!(amm.get("asset2_frozen"), Some(&JsonValue::Bool(true)));
}
