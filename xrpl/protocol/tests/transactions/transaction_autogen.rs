use basics::base_uint::Uint256;
use protocol::{
    AccountID, ClawbackBuilder, HashPrefix, JsonOptions, KeyType, LoanSetBuilder, NumberJsonInput,
    Payment, PaymentBuilder, PaymentChannelCreateBuilder, SOEStyle, SOETxMPTIssue, SOElement,
    SOTemplate, STAmount, STArray, STObject, STXChainBridge, SetFeeBuilder, TransactionBase,
    TxType, XRPAmount, build_multi_signing_data, derive_public_key, get_field_by_symbol,
    normalized_parts_from_json_input, validate_st_object, xrp_issue,
};

fn account(value: u64) -> AccountID {
    AccountID::from_u64(value)
}

fn fee_amount() -> STAmount {
    STAmount::from_xrp_amount(XRPAmount::from_drops(10))
}

fn signing_keys() -> (protocol::PublicKey, protocol::SecretKey) {
    let secret = protocol::SecretKey::from_bytes([7u8; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    (public, secret)
}

#[test]
fn payment_builder_signs_and_exposes_typed_fields() {
    let (public, secret) = signing_keys();
    let tx = PaymentBuilder::new(
        account(1),
        account(2),
        STAmount::from_xrp_amount(XRPAmount::from_drops(25)),
        Some(7),
        Some(fee_amount()),
    )
    .set_source_tag(55)
    .build(&public, &secret)
    .expect("payment");

    assert_eq!(tx.get_transaction_type(), TxType::PAYMENT);
    assert_eq!(tx.get_account(), account(1));
    assert_eq!(tx.get_destination(), account(2));
    assert_eq!(tx.get_sequence(), 7);
    assert_eq!(tx.get_source_tag(), Some(55));
    assert_eq!(tx.get_signing_pub_key(), public.as_bytes().to_vec());
    assert_eq!(tx.get_credential_ids(), None);
    assert_eq!(public.to_hex(), format!("{}", public));
    assert_eq!(
        format!("{}", public),
        basics::str_hex::str_hex(*public.as_bytes())
    );

    let blob = tx.as_sttx().get_json_binary(JsonOptions::NONE, true);
    assert!(matches!(blob, protocol::JsonValue::Object(_)));
}

#[test]
fn wrapper_rejects_wrong_transaction_type() {
    let (public, secret) = signing_keys();
    let clawback = ClawbackBuilder::new(
        account(1),
        STAmount::from_xrp_amount(XRPAmount::from_drops(5)),
        Some(1),
        Some(fee_amount()),
    )
    .build(&public, &secret)
    .expect("clawback");

    let wrong = Payment::new(clawback.tx().clone());
    assert_eq!(wrong.unwrap_err(), "Invalid transaction type for Payment");
}

#[test]
fn xchain_commit_builder_round_trips_typed_bridge() {
    let (public, secret) = signing_keys();
    let bridge = STXChainBridge::from_parts(account(10), xrp_issue(), account(20), xrp_issue());
    let tx = protocol::XChainCommitBuilder::new(
        account(1),
        bridge.clone(),
        9,
        STAmount::from_xrp_amount(XRPAmount::from_drops(50)),
        Some(3),
        Some(fee_amount()),
    )
    .set_other_chain_destination(account(30))
    .build(&public, &secret)
    .expect("xchain commit");

    assert_eq!(tx.get_x_chain_bridge(), bridge);
    assert_eq!(tx.get_x_chain_claim_id(), 9);
    assert_eq!(tx.get_other_chain_destination(), Some(account(30)));
}

#[test]
fn payment_channel_create_builder_preserves_raw_public_key_bytes() {
    let (public, secret) = signing_keys();
    let tx = PaymentChannelCreateBuilder::new(
        account(1),
        account(2),
        STAmount::from_xrp_amount(XRPAmount::from_drops(25)),
        60,
        public.as_bytes(),
        Some(11),
        Some(fee_amount()),
    )
    .build(&public, &secret)
    .expect("payment channel create");

    assert_eq!(tx.get_public_key(), public.as_bytes().to_vec());
    assert_eq!(tx.get_destination(), account(2));
    assert_eq!(tx.get_settle_delay(), 60);
}

#[test]
fn loan_set_builder_round_trips_lending_fields() {
    let (public, secret) = signing_keys();
    let principal_requested = protocol::STNumber::with_field(
        get_field_by_symbol("sfPrincipalRequested"),
        normalized_parts_from_json_input(NumberJsonInput::UInt(17)).expect("number"),
    );
    let loan_broker_id = Uint256::from_u64(9);
    let tx = LoanSetBuilder::new(
        account(1),
        loan_broker_id,
        principal_requested,
        Some(5),
        Some(fee_amount()),
    )
    .build(&public, &secret)
    .expect("loan set");

    assert_eq!(tx.get_loan_broker_id(), loan_broker_id);
    assert_eq!(tx.get_principal_requested(), principal_requested);
}

#[test]
fn transaction_base_validate_short_circuits_pseudo_transactions() {
    let (public, secret) = signing_keys();
    let tx = SetFeeBuilder::new(account(1), Some(1), Some(fee_amount()))
        .build(&public, &secret)
        .expect("set fee");

    let mut reason = String::new();
    assert!(TransactionBase::new(tx.tx().clone()).validate(&mut reason));
    assert!(reason.is_empty());
}

#[test]
fn transaction_base_validate_rejects_bad_memo_content() {
    let (public, secret) = signing_keys();
    let mut memo = STObject::new(get_field_by_symbol("sfMemo"));
    memo.set_field_vl(get_field_by_symbol("sfMemoType"), &[0x01]);
    let mut memos = STArray::new(get_field_by_symbol("sfMemos"));
    memos.push_back(memo);

    let tx = PaymentBuilder::new(
        account(1),
        account(2),
        STAmount::from_xrp_amount(XRPAmount::from_drops(25)),
        Some(7),
        Some(fee_amount()),
    )
    .set_memos(memos)
    .build(&public, &secret)
    .expect("payment");

    let mut reason = String::new();
    assert!(!TransactionBase::new(tx.tx().clone()).validate(&mut reason));
    assert_eq!(
        reason,
        "The MemoType and MemoFormat fields may only contain characters that are allowed in URLs under RFC 3986."
    );
}

#[test]
fn validate_st_object_rejects_mpt_in_not_supported_field() {
    let element = SOElement::new_with_mpt(
        get_field_by_symbol("sfAmount"),
        SOEStyle::Required,
        SOETxMPTIssue::NotSupported,
    )
    .expect("template element");
    let template = SOTemplate::new(vec![element], Vec::new()).expect("template");
    let mut object = STObject::new(get_field_by_symbol("sfTransaction"));
    object.set_field_amount(
        get_field_by_symbol("sfAmount"),
        STAmount::from_mpt_amount(
            get_field_by_symbol("sfAmount"),
            protocol::MPTAmount::from_value(5),
            protocol::MPTIssue::new(protocol::MPTID::from_u64(9)),
        ),
    );

    assert!(!validate_st_object(&object, &template));
}

#[test]
fn multi_signing_data_appends_signer_to_shared_prefix() {
    let object = STObject::new(get_field_by_symbol("sfTransaction"));
    let serializer = build_multi_signing_data(&object, account(99));
    assert_eq!(
        serializer.data()[..4],
        HashPrefix::TxMultiSign.as_u32().to_be_bytes()
    );
    assert!(serializer.size() > 4);
}
