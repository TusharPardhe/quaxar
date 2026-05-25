use protocol::{
    AccountID, CurrentTransactionRulesGuard, IOUAmount, Issue, MPTAmount, MPTIssue, Rules,
    SOEStyle, SOETxMPTIssue, SOElement, SOTemplate, STAmount, STObject, STXChainBridge,
    XChainBridgeChainType, currency_from_string, feature_amm, fix_inner_obj_template2,
    get_field_by_symbol, make_mpt_id, parse_base58_account_id, register_runtime_sfield, sf_generic,
    validate_st_object,
};

fn account(value: &str) -> AccountID {
    parse_base58_account_id(value).expect("account should parse")
}

#[test]
fn protocol_stobject_apply_template_enforces_required_default_and_discardable_rules() {
    let account_field = get_field_by_symbol("sfAccount");
    let flags_field = get_field_by_symbol("sfFlags");
    let sequence_field = get_field_by_symbol("sfSequence");
    let runtime_discardable = register_runtime_sfield(
        "sfRuntimeParityDiscardableObject",
        protocol::SerializedTypeId::UInt32,
        9101,
        "RuntimeParityDiscardableObject",
        protocol::SField::S_MD_DEFAULT,
        protocol::IsSigning::Yes,
        None,
    )
    .expect("runtime field should register");

    let template = SOTemplate::new(
        vec![
            SOElement::new(account_field, SOEStyle::Required).expect("required element"),
            SOElement::new(flags_field, SOEStyle::Default).expect("default element"),
        ],
        vec![SOElement::new(sequence_field, SOEStyle::Optional).expect("optional element")],
    )
    .expect("template should build");

    let mut object = STObject::new(sf_generic());
    object.set_account_id(account_field, account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh"));
    object.set_field_u32(runtime_discardable, 99);
    object.apply_template(&template);

    assert_eq!(object.get_count(), 3);
    assert!(object.is_field_present(account_field));
    assert!(!object.is_field_present(flags_field));
    assert!(!object.is_field_present(sequence_field));
    assert!(!object.is_field_present(runtime_discardable));

    let missing_required = std::panic::catch_unwind(|| {
        let mut candidate = STObject::new(sf_generic());
        candidate.apply_template(&template);
    });
    assert!(
        missing_required.is_ok(),
        "C++ inserts defaults for missing required fields"
    );

    let explicit_default = std::panic::catch_unwind(|| {
        let mut candidate = STObject::new(sf_generic());
        candidate.set_account_id(account_field, account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh"));
        candidate.set_field_u32(flags_field, 0);
        candidate.apply_template(&template);
    });
    assert!(
        explicit_default.is_ok(),
        "C++ silently accepts explicit default-valued fields"
    );
}

#[test]
fn protocol_stobject_typed_accessors_round_trip_issue_amount_and_xchain_bridge() {
    let issuer = account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh");
    let amount = STAmount::from_iou_amount(
        get_field_by_symbol("sfAmount"),
        IOUAmount::from_parts(2_500_000_000_000_000, -14).expect("amount"),
        Issue::new(currency_from_string("USD"), issuer),
    );
    let bridge = STXChainBridge::from_parts(
        issuer,
        protocol::xrp_issue(),
        account("r3kmLJN5D28dHuH8vZNUZpMC43pEHpaocV"),
        Issue::new(currency_from_string("USD"), issuer),
    );

    let mut object = STObject::new(sf_generic());
    object.set_field_amount(get_field_by_symbol("sfAmount"), amount.clone());
    object.set_field_issue(
        get_field_by_symbol("sfAsset"),
        protocol::STIssue::new_with_asset(
            get_field_by_symbol("sfAsset"),
            bridge.locking_chain_issue(),
        ),
    );
    object.set_field_xchain_bridge(get_field_by_symbol("sfXChainBridge"), bridge.clone());

    assert_eq!(
        object.get_field_amount(get_field_by_symbol("sfAmount")),
        amount
    );
    assert_eq!(
        object
            .get_field_issue(get_field_by_symbol("sfAsset"))
            .asset(),
        bridge.locking_chain_issue()
    );
    assert_eq!(
        object.get_field_xchain_bridge(get_field_by_symbol("sfXChainBridge")),
        bridge
    );
    assert_eq!(
        object
            .get_field_xchain_bridge(get_field_by_symbol("sfXChainBridge"))
            .door(XChainBridgeChainType::Issuing),
        account("r3kmLJN5D28dHuH8vZNUZpMC43pEHpaocV")
    );
}

#[test]
fn protocol_stobject_inner_object_templates_follow_current_rule_gates() {
    let signer_entry = get_field_by_symbol("sfSignerEntry");
    let vote_entry = get_field_by_symbol("sfVoteEntry");

    let default_rules = STObject::make_inner_object(signer_entry);
    assert_eq!(default_rules.get_count(), 3);

    let amm_only = CurrentTransactionRulesGuard::new(Rules::new([feature_amm()]));
    assert_eq!(STObject::make_inner_object(signer_entry).get_count(), 0);
    drop(amm_only);

    let fixed_signer =
        CurrentTransactionRulesGuard::new(Rules::new([feature_amm(), fix_inner_obj_template2()]));
    assert_eq!(STObject::make_inner_object(signer_entry).get_count(), 3);
    drop(fixed_signer);

    let amm_vote = CurrentTransactionRulesGuard::new(Rules::new([
        feature_amm(),
        protocol::fix_inner_obj_template(),
    ]));
    assert_eq!(STObject::make_inner_object(vote_entry).get_count(), 3);
    drop(amm_vote);
}

#[test]
fn protocol_stobject_validation_uses_template_required_and_mpt_support_rules() {
    let amount_field = get_field_by_symbol("sfAmount");
    let template = SOTemplate::new(
        vec![
            SOElement::new_with_mpt(
                amount_field,
                SOEStyle::Required,
                SOETxMPTIssue::NotSupported,
            )
            .expect("amount element"),
        ],
        Vec::new(),
    )
    .expect("template");

    let mut missing = STObject::new(get_field_by_symbol("sfTransaction"));
    assert!(!validate_st_object(&missing, &template));

    missing.set_field_amount(
        amount_field,
        STAmount::from_mpt_amount(
            amount_field,
            MPTAmount::from_value(5),
            MPTIssue::new(make_mpt_id(
                9,
                account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh"),
            )),
        ),
    );
    assert!(!validate_st_object(&missing, &template));

    missing.set_field_amount(
        amount_field,
        STAmount::from_iou_amount(
            amount_field,
            IOUAmount::from_parts(1_000_000_000_000_000, -15).expect("iou"),
            Issue::new(
                currency_from_string("USD"),
                account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh"),
            ),
        ),
    );
    assert!(validate_st_object(&missing, &template));
}
