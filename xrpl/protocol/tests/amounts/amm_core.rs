use protocol::{
    AccountID, Currency, Issue, Rules, STAmount, STObject, Ter, amm_auction_time_slot, amm_enabled,
    amm_lpt_currency, amm_lpt_issue, bad_currency, currency_from_string, feature_amm,
    feature_universal_number, get_field_by_symbol, invalid_amm_amount, invalid_amm_asset_pair,
};

fn sample_account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

#[test]
fn amm_enabled_requires_both_amm_and_universal_number() {
    assert!(!amm_enabled(&Rules::new([])));
    assert!(!amm_enabled(&Rules::new([feature_amm()])));
    assert!(!amm_enabled(&Rules::new([feature_universal_number()])));
    assert!(amm_enabled(&Rules::new([
        feature_amm(),
        feature_universal_number(),
    ])));
}

#[test]
fn amm_lpt_currency_order_independence_and_prefix_rules() {
    let usd = currency_from_string("USD");
    let eur = currency_from_string("EUR");
    let amm_account = sample_account(0x41);

    let left = amm_lpt_currency(usd, eur);
    let right = amm_lpt_currency(eur, usd);
    let issue = amm_lpt_issue(usd, eur, amm_account);

    assert_eq!(left, right);
    assert_eq!(left.data()[0], 0x03);
    assert_eq!(issue, Issue::new(left, amm_account));
}

#[test]
fn invalid_amm_asset_pair_and_amount_match_cpp_error_codes() {
    let issuer = sample_account(0x51);
    let other = sample_account(0x52);
    let usd = currency_from_string("USD");
    let eur = currency_from_string("EUR");
    let usd_issue = Issue::new(usd, issuer);
    let eur_issue = Issue::new(eur, other);

    assert_eq!(
        invalid_amm_asset_pair(usd_issue, usd_issue, None),
        Ter::TEM_BAD_AMM_TOKENS
    );
    assert_eq!(
        invalid_amm_asset_pair(Issue::new(bad_currency(), issuer), eur_issue, None),
        Ter::TEM_BAD_CURRENCY
    );
    assert_eq!(
        invalid_amm_asset_pair(Issue::new(Currency::zero(), issuer), eur_issue, None),
        Ter::TEM_BAD_ISSUER
    );
    assert_eq!(
        invalid_amm_asset_pair(
            usd_issue,
            eur_issue,
            Some((usd_issue, Issue::new(currency_from_string("JPY"), other))),
        ),
        Ter::TEM_BAD_AMM_TOKENS
    );

    let negative = STAmount::new_with_asset(get_field_by_symbol("sfAmount"), usd_issue, 1, 0, true);
    let zero = STAmount::new_with_asset(get_field_by_symbol("sfAmount"), usd_issue, 0, 0, false);
    assert_eq!(
        invalid_amm_amount(&negative, None, false),
        Ter::TEM_BAD_AMOUNT
    );
    assert_eq!(invalid_amm_amount(&zero, None, false), Ter::TEM_BAD_AMOUNT);
    assert_eq!(invalid_amm_amount(&zero, None, true), Ter::TES_SUCCESS);
}

#[test]
fn amm_auction_time_slot_boundaries() {
    let mut slot = STObject::new(get_field_by_symbol("sfAuctionSlot"));
    let expiration = 123_456u32;
    slot.set_field_u32(get_field_by_symbol("sfExpiration"), expiration);

    let start = expiration - protocol::TOTAL_TIME_SLOT_SECS;
    assert_eq!(amm_auction_time_slot(u64::from(start), &slot), Some(0));
    assert_eq!(
        amm_auction_time_slot(u64::from(expiration - 1), &slot),
        Some(19)
    );
    assert_eq!(amm_auction_time_slot(u64::from(expiration), &slot), None);
    assert_eq!(amm_auction_time_slot(u64::from(start - 1), &slot), None);
}
