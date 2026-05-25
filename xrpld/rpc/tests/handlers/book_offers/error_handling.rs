//! book offers tests part 1.

use super::*;

#[test]
fn book_offers_rejects_busy_missing_invalid_and_malformed() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        client_jobs: 201,
    };
    let runtime = FakeRuntime::default();

    let busy = run(JsonValue::Object(Default::default()), &source, &runtime);
    let busy = result_object(busy);
    assert_eq!(
        busy.get("error"),
        Some(&JsonValue::String("tooBusy".to_owned()))
    );
    assert_eq!(busy.get("error_code"), Some(&JsonValue::Signed(9)));
    assert_eq!(
        busy.get("error_message"),
        Some(&JsonValue::String(
            "The server is too busy to help you now.".to_owned()
        ))
    );

    let source = FakeSource {
        ledger: Some(closed_ledger()),
        client_jobs: 0,
    };

    let missing = run(JsonValue::Object(Default::default()), &source, &runtime);
    let missing = result_object(missing);
    assert_eq!(
        missing.get("error_message"),
        Some(&JsonValue::String("Missing field 'taker_pays'.".to_owned()))
    );

    let null_pay = run(
        object([
            ("taker_pays", JsonValue::Null),
            (
                "taker_gets",
                object([("currency", JsonValue::String("USD".to_owned()))]),
            ),
        ]),
        &source,
        &runtime,
    );
    let null_pay = result_object(null_pay);
    assert_eq!(
        null_pay.get("error_message"),
        Some(&JsonValue::String(
            "Missing field 'taker_pays.currency'.".to_owned()
        ))
    );

    let invalid_object = run(
        object([
            (
                "taker_pays",
                object([("currency", JsonValue::String("XRP".to_owned()))]),
            ),
            ("taker_gets", JsonValue::String("not an object".to_owned())),
        ]),
        &source,
        &runtime,
    );
    let invalid_object = result_object(invalid_object);
    assert_eq!(
        invalid_object.get("error_message"),
        Some(&JsonValue::String(
            "Invalid field 'taker_gets', not object.".to_owned()
        ))
    );

    let bad_taker = run(
        object([
            ("taker", JsonValue::Unsigned(1)),
            (
                "taker_pays",
                object([("currency", JsonValue::String("XRP".to_owned()))]),
            ),
            (
                "taker_gets",
                object([
                    ("currency", JsonValue::String("USD".to_owned())),
                    ("issuer", JsonValue::String(to_base58(sample_account(0x11)))),
                ]),
            ),
        ]),
        &source,
        &runtime,
    );
    let bad_taker = result_object(bad_taker);
    assert_eq!(
        bad_taker.get("error_message"),
        Some(&JsonValue::String(
            "Invalid field 'taker', not string.".to_owned()
        ))
    );

    let malformed_domain = run(
        object([
            ("taker", JsonValue::String(to_base58(sample_account(0x22)))),
            ("domain", JsonValue::String("badString".to_owned())),
            (
                "taker_pays",
                object([("currency", JsonValue::String("XRP".to_owned()))]),
            ),
            (
                "taker_gets",
                object([
                    ("currency", JsonValue::String("USD".to_owned())),
                    ("issuer", JsonValue::String(to_base58(sample_account(0x33)))),
                ]),
            ),
        ]),
        &source,
        &runtime,
    );
    let malformed_domain = result_object(malformed_domain);
    assert_eq!(
        malformed_domain.get("error"),
        Some(&JsonValue::String("domainMalformed".to_owned()))
    );
    assert_eq!(
        malformed_domain.get("error_message"),
        Some(&JsonValue::String("Unable to parse domain.".to_owned()))
    );

    let bad_market = run(
        object([
            ("taker", JsonValue::String(to_base58(sample_account(0x44)))),
            (
                "taker_pays",
                object([
                    ("currency", JsonValue::String("USD".to_owned())),
                    ("issuer", JsonValue::String(to_base58(sample_account(0x55)))),
                ]),
            ),
            (
                "taker_gets",
                object([
                    ("currency", JsonValue::String("USD".to_owned())),
                    ("issuer", JsonValue::String(to_base58(sample_account(0x55)))),
                ]),
            ),
        ]),
        &source,
        &runtime,
    );
    let bad_market = result_object(bad_market);
    assert_eq!(
        bad_market.get("error"),
        Some(&JsonValue::String("badMarket".to_owned()))
    );
    assert_eq!(bad_market.get("error_code"), Some(&JsonValue::Signed(42)));
    assert_eq!(
        bad_market.get("error_message"),
        Some(&JsonValue::String("No such market.".to_owned()))
    );

    let bad_limit = run(
        object([
            ("limit", JsonValue::Unsigned(0)),
            ("taker", JsonValue::String(to_base58(sample_account(0x66)))),
            (
                "taker_pays",
                object([("currency", JsonValue::String("XRP".to_owned()))]),
            ),
            (
                "taker_gets",
                object([
                    ("currency", JsonValue::String("USD".to_owned())),
                    ("issuer", JsonValue::String(to_base58(sample_account(0x77)))),
                ]),
            ),
        ]),
        &source,
        &runtime,
    );
    let bad_limit = result_object(bad_limit);
    assert_eq!(
        bad_limit.get("error_message"),
        Some(&JsonValue::String("Invalid field 'limit'.".to_owned()))
    );
}

#[test]
fn book_offers_delegates_page_shaping_and_preserves_ledger() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        client_jobs: 0,
    };
    let runtime = FakeRuntime::default();
    let taker = sample_account(0xAB);
    let issue_issuer = sample_account(0xCD);
    let domain =
        Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
            .expect("domain");

    let response = run(
        object([
            ("domain", JsonValue::String(domain.to_string())),
            ("marker", JsonValue::String("MyMarker".to_owned())),
            ("proof", JsonValue::Bool(false)),
            ("taker", JsonValue::String(to_base58(taker))),
            (
                "taker_pays",
                object([("currency", JsonValue::String("XRP".to_owned()))]),
            ),
            (
                "taker_gets",
                object([
                    ("currency", JsonValue::String("USD".to_owned())),
                    ("issuer", JsonValue::String(to_base58(issue_issuer))),
                ]),
            ),
        ]),
        &source,
        &runtime,
    );

    let response = result_object(response);
    assert_eq!(
        response.get("ledger_hash"),
        Some(&JsonValue::String(closed_ledger().hash.to_string()))
    );
    assert_eq!(
        response.get("ledger_index"),
        Some(&JsonValue::Unsigned(u64::from(closed_ledger().seq)))
    );
    assert_eq!(response.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        response.get("offers"),
        Some(&JsonValue::Array(vec![JsonValue::Object(BTreeMap::from(
            [(
                "shape".to_owned(),
                JsonValue::String("delegated".to_owned()),
            )]
        ))]))
    );

    let call = runtime
        .call
        .borrow()
        .clone()
        .expect("runtime should be called");
    assert_eq!(call.ledger, closed_ledger());
    let expected_book = Book::new(
        Issue::new(xrp_currency(), xrp_account()),
        Issue::new(currency_from_string("USD"), issue_issuer),
        Some(domain),
    );
    assert_eq!(call.book, expected_book);
    assert_eq!(call.taker, taker);
    assert!(
        call.proof,
        "proof should be treated as present, even when false"
    );
    assert_eq!(call.limit, rpc::tuning::Tuning::BOOK_OFFERS.r_default);
    assert_eq!(call.marker, JsonValue::String("MyMarker".to_owned()));
}

#[test]
fn book_offers_missing_taker_pays_error() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        client_jobs: 0,
    };
    let runtime = FakeRuntime::default();

    let result = run(
        object([(
            "taker_gets",
            object([("currency", JsonValue::String("USD".to_owned()))]),
        )]),
        &source,
        &runtime,
    );
    let result = result_object(result);
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );
    assert_eq!(
        result.get("error_message"),
        Some(&JsonValue::String("Missing field 'taker_pays'.".to_owned()))
    );
    assert_eq!(result.get("error_code"), Some(&JsonValue::Signed(31)));
}

#[test]
fn book_offers_missing_taker_gets_error() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        client_jobs: 0,
    };
    let runtime = FakeRuntime::default();

    let result = run(
        object([(
            "taker_pays",
            object([("currency", JsonValue::String("XRP".to_owned()))]),
        )]),
        &source,
        &runtime,
    );
    let result = result_object(result);
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );
    assert_eq!(
        result.get("error_message"),
        Some(&JsonValue::String("Missing field 'taker_gets'.".to_owned()))
    );
}

#[test]
fn book_offers_missing_currency_in_taker_pays() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        client_jobs: 0,
    };
    let runtime = FakeRuntime::default();

    let result = run(
        object([
            ("taker_pays", object([])),
            (
                "taker_gets",
                object([("currency", JsonValue::String("USD".to_owned()))]),
            ),
        ]),
        &source,
        &runtime,
    );
    let result = result_object(result);
    assert_eq!(
        result.get("error_message"),
        Some(&JsonValue::String(
            "Missing field 'taker_pays.currency'.".to_owned()
        ))
    );
}

#[test]
fn book_offers_same_currency_same_issuer_bad_market() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        client_jobs: 0,
    };
    let runtime = FakeRuntime::default();
    let issuer = sample_account(0x11);

    let result = run(
        object([
            (
                "taker_pays",
                object([
                    ("currency", JsonValue::String("USD".to_owned())),
                    ("issuer", JsonValue::String(to_base58(issuer))),
                ]),
            ),
            (
                "taker_gets",
                object([
                    ("currency", JsonValue::String("USD".to_owned())),
                    ("issuer", JsonValue::String(to_base58(issuer))),
                ]),
            ),
        ]),
        &source,
        &runtime,
    );
    let result = result_object(result);
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("badMarket".to_owned()))
    );
    assert_eq!(result.get("error_code"), Some(&JsonValue::Signed(42)));
    assert_eq!(
        result.get("error_message"),
        Some(&JsonValue::String("No such market.".to_owned()))
    );
}

#[test]
fn book_offers_xrp_to_xrp_bad_market() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        client_jobs: 0,
    };
    let runtime = FakeRuntime::default();

    let result = run(
        object([
            (
                "taker_pays",
                object([("currency", JsonValue::String("XRP".to_owned()))]),
            ),
            (
                "taker_gets",
                object([("currency", JsonValue::String("XRP".to_owned()))]),
            ),
        ]),
        &source,
        &runtime,
    );
    let result = result_object(result);
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("badMarket".to_owned()))
    );
}
