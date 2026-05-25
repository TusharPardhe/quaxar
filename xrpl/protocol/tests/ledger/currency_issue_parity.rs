use basics::str_hex::str_hex;
use protocol::{
    Asset, Currency, CurrentTransactionRulesGuard, JsonOptions, JsonValue, MPTIssue, Rules,
    STAccount, STCurrency, STIssue, STObject, STUInt16, STVar, Serializer, StBase, asset_from_json,
    asset_to_string, currency_from_string, currency_to_string, feature_amm, fix_inner_obj_template,
    fix_inner_obj_template2, get_field_by_symbol, issue_from_json, make_mpt_id, no_account,
    parse_base58_account_id, to_base58, xrp_issue,
};

#[test]
fn currency_text_json_and_wire_match_cpp_rules() {
    let field = get_field_by_symbol("sfCurrency");
    let usd = currency_from_string("USD");
    let st_currency = STCurrency::new_with_currency(field, usd);

    assert_eq!(currency_to_string(usd), "USD");
    assert_eq!(st_currency.text(), "USD");
    assert_eq!(
        st_currency.json(JsonOptions::NONE),
        JsonValue::String("USD".into())
    );
    assert!(!st_currency.is_default());

    let mut serializer = Serializer::default();
    st_currency.add(&mut serializer);
    assert_eq!(
        str_hex(serializer.data()),
        "0000000000000000000000005553440000000000"
    );

    let parsed = Currency::from_slice(serializer.data()).expect("currency width");
    assert_eq!(parsed, usd);
}

#[test]
fn account_id_base58_round_trips_for_issue_json() {
    let account =
        parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").expect("genesis account");
    assert_eq!(to_base58(account), "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh");
    assert_ne!(account, no_account());
}

#[test]
fn issue_json_text_and_wire_cover_xrp_iou_and_mpt_shapes() {
    let field = get_field_by_symbol("sfAsset");
    let issuer =
        parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").expect("genesis account");

    let xrp = STIssue::new_with_asset(field, xrp_issue());
    assert!(xrp.is_default());
    assert_eq!(xrp.text(), "XRP");
    let mut xrp_ser = Serializer::default();
    xrp.add(&mut xrp_ser);
    assert_eq!(
        str_hex(xrp_ser.data()),
        "0000000000000000000000000000000000000000"
    );

    let iou_json = JsonValue::Object(
        [
            ("currency".to_string(), JsonValue::String("USD".into())),
            ("issuer".to_string(), JsonValue::String(to_base58(issuer))),
        ]
        .into_iter()
        .collect(),
    );
    let iou_issue = issue_from_json(&iou_json).expect("iou issue");
    let iou = STIssue::new_with_asset(field, iou_issue);
    assert_eq!(iou.text(), "USD/rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh");
    assert_eq!(
        asset_to_string(Asset::Issue(iou_issue)),
        "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh/USD"
    );
    let mut iou_ser = Serializer::default();
    iou.add(&mut iou_ser);
    assert_eq!(
        str_hex(iou_ser.data()),
        "0000000000000000000000005553440000000000B5F762798A53D543A014CAF8B297CFF8F2F937E8"
    );

    let mpt_id = make_mpt_id(7, issuer);
    let mpt = STIssue::new_with_asset(field, MPTIssue::new(mpt_id));
    let mut mpt_ser = Serializer::default();
    mpt.add(&mut mpt_ser);
    assert_eq!(
        str_hex(mpt_ser.data()),
        "B5F762798A53D543A014CAF8B297CFF8F2F937E8000000000000000000000000000000000000000100000007"
    );

    let parsed_iou = STIssue::from_serial_iter(
        &mut protocol::SerialIter::new(iou_ser.data()),
        get_field_by_symbol("sfAsset"),
    );
    assert_eq!(parsed_iou, iou);

    let parsed_mpt = STIssue::from_serial_iter(
        &mut protocol::SerialIter::new(mpt_ser.data()),
        get_field_by_symbol("sfAsset"),
    );
    assert_eq!(parsed_mpt, mpt);

    let mpt_json = JsonValue::Object(
        [(
            "mpt_issuance_id".to_string(),
            JsonValue::String(mpt_id.to_string()),
        )]
        .into_iter()
        .collect(),
    );
    assert_eq!(
        asset_from_json(&mpt_json).expect("mpt asset"),
        Asset::MPTIssue(MPTIssue::new(mpt_id))
    );
}

#[test]
fn inner_object_templates_follow_cpp_gating_and_application_rules() {
    let signer_entry = get_field_by_symbol("sfSignerEntry");
    let vote_entry = get_field_by_symbol("sfVoteEntry");

    let default_rules_object = STObject::make_inner_object(signer_entry);
    assert_eq!(default_rules_object.get_count(), 3);

    let guard = CurrentTransactionRulesGuard::new(Rules::new([feature_amm()]));
    let ungated_signer = STObject::make_inner_object(signer_entry);
    assert_eq!(ungated_signer.get_count(), 0);
    drop(guard);

    let amm_guard =
        CurrentTransactionRulesGuard::new(Rules::new([feature_amm(), fix_inner_obj_template()]));
    let amm_vote = STObject::make_inner_object(vote_entry);
    assert_eq!(amm_vote.get_count(), 3);
    drop(amm_guard);

    let general_guard = CurrentTransactionRulesGuard::new(Rules::new([fix_inner_obj_template2()]));
    let general_signer = STObject::make_inner_object(signer_entry);
    assert_eq!(general_signer.get_count(), 3);
    drop(general_guard);

    let mut object = STObject::new(signer_entry);
    object.emplace_back(STVar::new(STAccount::with_field(get_field_by_symbol(
        "sfAccount",
    ))));
    object.emplace_back(STVar::new(STUInt16::with_field(
        get_field_by_symbol("sfSignerWeight"),
        1,
    )));
    object.apply_template_from_sfield(signer_entry);
    assert_eq!(object.get_count(), 3);
    assert!(!object.is_field_present(get_field_by_symbol("sfWalletLocator")));
}
