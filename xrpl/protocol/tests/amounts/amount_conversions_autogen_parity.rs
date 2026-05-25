use basics::number::{MantissaScale, NumberParts as RuntimeNumber, RoundingMode};
use protocol::{
    Asset, IOUAmount, Issue, LedgerEntryType, LedgerFormats, MPTAmount, MPTIssue, PathAsset,
    STAmount, STPath, STPathElement, STPathSet, TxFormats, TxType, XRPAmount, currency_from_string,
    get_asset, get_field_by_symbol, make_mpt_id, no_issue, to_amount, to_amount_from_number,
    to_max_amount, to_st_amount, to_st_amount_with_asset, xrp_issue,
};

fn runtime_number(mantissa: i64, exponent: i32) -> RuntimeNumber {
    RuntimeNumber::try_from_external_parts(mantissa, exponent, MantissaScale::Large)
        .expect("runtime number")
}

#[test]
fn amount_conversions_round_trip_all_supported_amount_kinds() {
    let issuer =
        protocol::parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").expect("account");
    let issue = Issue::new(currency_from_string("USD"), issuer);
    let mpt_issue = MPTIssue::new(make_mpt_id(9, issuer));

    let iou = IOUAmount::from_parts(12_345_678_901_234_567, -4).expect("iou");
    let xrp = XRPAmount::from_drops(25);
    let mpt = MPTAmount::from_value(44);

    let iou_amount = to_st_amount_with_asset(iou, Asset::Issue(issue));
    let xrp_amount = to_st_amount(xrp);
    let mpt_amount = to_st_amount_with_asset(mpt, Asset::MPTIssue(mpt_issue));

    assert_eq!(to_amount::<IOUAmount, _>(&iou_amount), iou);
    assert_eq!(to_amount::<XRPAmount, _>(&xrp_amount), xrp);
    assert_eq!(to_amount::<MPTAmount, _>(&mpt_amount), mpt);
    assert_eq!(get_asset(&iou_amount), Asset::Issue(issue));
    assert_eq!(get_asset(&xrp_amount), Asset::Issue(xrp_issue()));
    assert_eq!(get_asset(&mpt_amount), Asset::MPTIssue(mpt_issue));
}

#[test]
fn amount_conversions_from_number_and_max_amount_follow_asset_kind() {
    let issuer =
        protocol::parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").expect("account");
    let issue = Issue::new(currency_from_string("USD"), issuer);
    let mpt_issue = MPTIssue::new(make_mpt_id(3, issuer));

    let iou_amount = to_amount_from_number::<STAmount>(
        Asset::Issue(issue),
        runtime_number(12345, -2),
        RoundingMode::ToNearest,
    )
    .expect("iou stamount");
    let xrp_amount = to_amount_from_number::<STAmount>(
        Asset::Issue(xrp_issue()),
        runtime_number(50, 0),
        RoundingMode::Upward,
    )
    .expect("xrp stamount");
    let mpt_amount = to_amount_from_number::<STAmount>(
        Asset::MPTIssue(mpt_issue),
        runtime_number(77, 0),
        RoundingMode::ToNearest,
    )
    .expect("mpt stamount");

    assert_eq!(
        to_amount::<IOUAmount, _>(&iou_amount),
        IOUAmount::from_parts(1234500000000000, -13).expect("canonicalized iou")
    );
    assert_eq!(
        to_amount::<XRPAmount, _>(&xrp_amount),
        XRPAmount::from_drops(50)
    );
    assert_eq!(
        to_amount::<MPTAmount, _>(&mpt_amount),
        MPTAmount::from_value(77)
    );

    assert_eq!(
        to_max_amount::<XRPAmount>(Asset::Issue(xrp_issue())).drops(),
        100_000_000_000_000_000
    );
    assert_eq!(
        to_max_amount::<MPTAmount>(Asset::MPTIssue(mpt_issue)).value(),
        protocol::MAX_MP_TOKEN_AMOUNT
    );
    assert_eq!(get_asset(&IOUAmount::new()), Asset::Issue(no_issue()));
}

#[test]
fn autogen_specs_expose_current_dex_and_mpt_fields() {
    let payment = TxFormats::get_instance()
        .find_by_type(TxType::PAYMENT)
        .expect("Payment format");
    assert!(
        payment
            .so_template()
            .elements()
            .iter()
            .any(|element| element.sfield() == get_field_by_symbol("sfDomainID"))
    );

    let amm = LedgerFormats::get_instance()
        .find_by_type(LedgerEntryType::AMM)
        .expect("AMM format");
    assert!(
        amm.so_template()
            .elements()
            .iter()
            .any(|element| element.sfield() == get_field_by_symbol("sfAsset2"))
    );

    let directory_node = LedgerFormats::get_instance()
        .find_by_type(LedgerEntryType::DirectoryNode)
        .expect("DirectoryNode format");
    assert!(
        directory_node
            .so_template()
            .elements()
            .iter()
            .any(|element| element.sfield() == get_field_by_symbol("sfDomainID"))
    );

    assert_eq!(
        get_field_by_symbol("sfAssetScale").symbol_name(),
        "sfAssetScale"
    );
    assert_eq!(
        get_field_by_symbol("sfMPTAmount").symbol_name(),
        "sfMPTAmount"
    );
}

#[test]
fn path_asset_conversion_matches_asset_token_kind() {
    let issuer =
        protocol::parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").expect("account");
    let issue = Issue::new(currency_from_string("USD"), issuer);
    let mpt_issue = MPTIssue::new(make_mpt_id(11, issuer));

    assert_eq!(
        PathAsset::from(Asset::Issue(issue)),
        PathAsset::from(issue.currency)
    );
    assert_eq!(
        PathAsset::from(Asset::MPTIssue(mpt_issue)),
        PathAsset::from(mpt_issue.mpt_id())
    );

    let mut path = STPath::new();
    path.push_back(STPathElement::inferred(
        issuer,
        PathAsset::from(mpt_issue.mpt_id()),
        protocol::AccountID::zero(),
        true,
    ));
    let mut set = STPathSet::new(get_field_by_symbol("sfPaths"));
    set.push_back(path);
    assert!(set[0][0].has_mpt());
}
