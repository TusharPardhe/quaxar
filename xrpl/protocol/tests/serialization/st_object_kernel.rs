use basics::{base_uint::Uint256, str_hex::str_hex};
use protocol::ter::trans_human;
use protocol::{
    AccountID, JsonOptions, JsonValue, STAccount, STAmount, STArray, STBlob, STInt32, STObject,
    STUInt8, STUInt16, STUInt32, STUInt64, STVar, STVector256, Serializer, StBase, Ter, TxType,
    calc_account_id, genesis_public_key, get_field_by_symbol, sf_generic, to_base58, trans_token,
};

#[test]
fn account_id_and_staccount_match_cpp_surface() {
    let zero = AccountID::zero();
    let genesis = calc_account_id(&genesis_public_key());

    assert_eq!(to_base58(zero), "rrrrrrrrrrrrrrrrrrrrrhoLvTp");
    assert_eq!(to_base58(genesis), "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh");

    let field = get_field_by_symbol("sfAccount");

    let default_account = STAccount::with_field(field);
    assert!(default_account.is_default());
    assert_eq!(default_account.text(), "");
    let mut default_ser = Serializer::default();
    default_account.add(&mut default_ser);
    assert_eq!(default_ser.data(), &[0x00]);

    let mut explicit_zero = STAccount::with_field(field);
    explicit_zero.set_value(AccountID::zero());
    assert!(!explicit_zero.is_default());
    assert_eq!(explicit_zero.text(), "rrrrrrrrrrrrrrrrrrrrrhoLvTp");
    let mut zero_ser = Serializer::default();
    explicit_zero.add(&mut zero_ser);
    assert_eq!(zero_ser.data()[0], 0x14);
    assert_eq!(&zero_ser.data()[1..], &[0u8; 20]);
}

#[test]
fn integer_json_and_text_match_cpp_special_cases() {
    let transaction_result = STUInt8::with_field(get_field_by_symbol("sfTransactionResult"), 0);
    assert_eq!(transaction_result.text(), trans_human(Ter::TES_SUCCESS));
    assert_eq!(
        transaction_result.json(JsonOptions::NONE),
        JsonValue::String(trans_token(Ter::TES_SUCCESS).to_string())
    );

    let tx_type = STUInt16::with_field(
        get_field_by_symbol("sfTransactionType"),
        TxType::PAYMENT.to_u16(),
    );
    assert_eq!(tx_type.text(), "Payment");
    assert_eq!(
        tx_type.json(JsonOptions::NONE),
        JsonValue::String("Payment".to_string())
    );

    let permission = STUInt32::with_field(get_field_by_symbol("sfPermissionValue"), 65_540);
    assert_eq!(permission.text(), "AccountDomainSet");
    assert_eq!(
        permission.json(JsonOptions::NONE),
        JsonValue::String("AccountDomainSet".to_string())
    );

    let hex_u64 = STUInt64::with_field(get_field_by_symbol("sfBaseFee"), 255);
    assert_eq!(
        hex_u64.json(JsonOptions::NONE),
        JsonValue::String("ff".to_string())
    );

    let decimal_u64 = STUInt64::with_field(get_field_by_symbol("sfMaximumAmount"), 255);
    assert_eq!(
        decimal_u64.json(JsonOptions::NONE),
        JsonValue::String("255".to_string())
    );

    let signed = STInt32::with_field(get_field_by_symbol("sfLedgerIndex"), -7);
    assert_eq!(signed.json(JsonOptions::NONE), JsonValue::Signed(-7));
}

#[test]
fn blob_and_vector256_round_trip_match_cpp_wire_shape() {
    let blob = STBlob::from_buffer(
        get_field_by_symbol("sfMemoData"),
        basics::buffer::Buffer::from_bytes(&[0xAA, 0xBB, 0xCC]),
    );
    assert_eq!(blob.text(), "AABBCC");
    let mut blob_ser = Serializer::default();
    blob.add(&mut blob_ser);
    assert_eq!(str_hex(blob_ser.data()), "03AABBCC");

    let first =
        Uint256::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
            .expect("test hash should parse");
    let second =
        Uint256::from_hex("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB")
            .expect("test hash should parse");
    let vector = STVector256::from_values(get_field_by_symbol("sfHashes"), vec![first, second]);
    assert_eq!(
        vector.json(JsonOptions::NONE),
        JsonValue::Array(vec![
            JsonValue::String(first.to_string()),
            JsonValue::String(second.to_string()),
        ])
    );
    let mut vector_ser = Serializer::default();
    vector.add(&mut vector_ser);
    assert_eq!(vector_ser.data()[0], 64);
    assert_eq!(&vector_ser.data()[1..33], first.data());
    assert_eq!(&vector_ser.data()[33..65], second.data());
}

#[test]
fn object_serialization_sorts_fields_and_appends_nested_terminators() {
    let mut object = STObject::new(sf_generic());
    object.emplace_back(STVar::new(STUInt32::with_field(
        get_field_by_symbol("sfSequence"),
        1,
    )));
    object.emplace_back(STVar::new(STUInt32::with_field(
        get_field_by_symbol("sfFlags"),
        2,
    )));

    let mut serializer = Serializer::default();
    object.add(&mut serializer);
    assert_eq!(str_hex(serializer.data()), "22000000022400000001");

    let mut inner = STObject::new(get_field_by_symbol("sfMemo"));
    inner.emplace_back(STVar::new(STUInt32::with_field(
        get_field_by_symbol("sfFlags"),
        9,
    )));

    let mut outer = STObject::new(sf_generic());
    outer.emplace_back(STVar::new(inner));

    let mut nested_ser = Serializer::default();
    outer.add(&mut nested_ser);
    assert_eq!(str_hex(nested_ser.data()), "EA2200000009E1");
}

#[test]
fn object_parse_rejects_duplicate_fields_and_array_terminators() {
    // Duplicate fields: deserializer returns false (failure) instead of panicking
    let mut duplicate = Serializer::default();
    duplicate.add_field_id(2, 4);
    duplicate.add32(1);
    duplicate.add_field_id(2, 4);
    duplicate.add32(2);
    duplicate.add_field_id(14, 1);

    let mut iter = protocol::SerialIter::new(duplicate.data());
    let obj = STObject::from_serial_iter(&mut iter, sf_generic(), 0);
    // Graceful failure: object is empty (fields cleared on duplicate detection)
    assert!(obj.empty());

    // Illegal end-of-array marker in object: returns false
    let mut illegal = Serializer::default();
    illegal.add_field_id(15, 1);

    let mut iter = protocol::SerialIter::new(illegal.data());
    let obj = STObject::from_serial_iter(&mut iter, sf_generic(), 0);
    assert!(obj.empty());
}

#[test]
fn array_parse_requires_object_entries_and_respects_terminators() {
    let mut good = Serializer::default();
    good.add_field_id(14, 10);
    good.add_field_id(2, 2);
    good.add32(7);
    good.add_field_id(14, 1);
    good.add_field_id(15, 1);

    let mut iter = protocol::SerialIter::new(good.data());
    let array = STArray::from_serial_iter(&mut iter, get_field_by_symbol("sfMemos"), 0);
    assert_eq!(array.iter().count(), 1);

    // Non-object in array: gracefully stops parsing (returns empty array)
    let mut non_object = Serializer::default();
    non_object.add_field_id(2, 2);
    non_object.add32(7);
    non_object.add_field_id(15, 1);

    let mut iter = protocol::SerialIter::new(non_object.data());
    let array = STArray::from_serial_iter(&mut iter, get_field_by_symbol("sfMemos"), 0);
    // Graceful failure: array is empty because first entry was not an object
    assert_eq!(array.iter().count(), 0);
}

#[test]
fn stvar_now_supports_amount() {
    let value = STVar::from_serialized_type(
        protocol::SerializedTypeId::Amount,
        get_field_by_symbol("sfBalance"),
    );
    assert_eq!(value.stype(), protocol::SerializedTypeId::Amount);

    let parsed = value
        .as_any()
        .downcast_ref::<STAmount>()
        .expect("amount type");
    assert!(parsed.is_default());
}

#[test]
fn typed_field_accessors_and_mutators_match_cpp_default_and_materialization_rules() {
    let format = protocol::LedgerFormats::get_instance()
        .find_by_type(protocol::LedgerEntryType::AccountRoot)
        .expect("account root format");
    let mut object =
        STObject::with_template(format.so_template(), get_field_by_symbol("sfLedgerEntry"));

    assert_eq!(
        object.get_field_h256(get_field_by_symbol("sfPreviousTxnID")),
        basics::base_uint::Uint256::zero()
    );
    assert_eq!(
        object.get_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq")),
        0
    );
    assert_eq!(
        object.get_account_id(get_field_by_symbol("sfAccount")),
        AccountID::zero()
    );

    let prev_tx = basics::base_uint::Uint256::from_array([0xAB; 32]);
    let account = calc_account_id(&genesis_public_key());
    object.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), prev_tx);
    object.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 99);
    object.set_account_id(get_field_by_symbol("sfAccount"), account);

    assert_eq!(
        object.get_field_h256(get_field_by_symbol("sfPreviousTxnID")),
        prev_tx
    );
    assert_eq!(
        object.get_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq")),
        99
    );
    assert_eq!(
        object.get_account_id(get_field_by_symbol("sfAccount")),
        account
    );

    object.make_field_absent(get_field_by_symbol("sfPreviousTxnID"));
    assert_eq!(
        object.get_field_h256(get_field_by_symbol("sfPreviousTxnID")),
        basics::base_uint::Uint256::zero()
    );
}

#[test]
fn flags_match_cpp_free_object_behavior() {
    let mut object = STObject::new(sf_generic());

    assert_eq!(object.get_flags(), 0);
    assert!(object.set_flag(0x10));
    assert_eq!(object.get_flags(), 0x10);
    assert!(object.is_flag(0x10));
    assert!(object.clear_flag(0x08));
    assert_eq!(object.get_flags(), 0x10);
    assert!(object.clear_flag(0x10));
    assert_eq!(object.get_flags(), 0);
    assert!(!object.is_flag(0x10));
}
