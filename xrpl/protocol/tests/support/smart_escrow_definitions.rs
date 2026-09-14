use basics::base_uint::Uint256;
use protocol::ter::trans_code;
use protocol::{
    FEATURE_LENDING_PROTOCOL_V1_2_NAME, FEATURE_SMART_ESCROW_NAME, FIX_CLEANUP_3_5_0_NAME,
    LedgerEntryType, LedgerFormats, RegisteredFeatureVote, SOEStyle, SerializedTypeId, Ter,
    TxFormats, TxType, feature_lending_protocol_v1_2, feature_smart_escrow, field_code,
    fix_cleanup_3_5_0, get_field_by_symbol, registered_feature, trans_human, trans_token,
};

#[test]
fn smart_escrow_amendment_definitions_are_golden_and_runtime_stays_disabled() {
    let smart_escrow =
        Uint256::from_hex("78ECD9CE17B0BF5B83BB3B275921FB5F5E0F672E9D24BD2E848B7C6277AE296E")
            .unwrap();
    let lending_v1_2 =
        Uint256::from_hex("5AE2B8F0B54FFB81E7B27BD7005C8401D9555E4F399369CA22D041F22815C824")
            .unwrap();
    let cleanup_3_5 =
        Uint256::from_hex("7300E10109D19BF1E87ACE63D3A79CD4ED6B9851C37D93ED94DE9BD41CF56835")
            .unwrap();

    assert_eq!(feature_smart_escrow(), smart_escrow);
    assert_eq!(feature_lending_protocol_v1_2(), lending_v1_2);
    assert_eq!(fix_cleanup_3_5_0(), cleanup_3_5);

    for id in [smart_escrow, lending_v1_2] {
        let feature = registered_feature(&id).expect("registered unsupported amendment");
        assert!(
            !feature.supported,
            "unsupported runtime must remain disabled"
        );
        assert_eq!(feature.vote, RegisteredFeatureVote::DefaultNo);
    }
    assert_eq!(
        registered_feature(&smart_escrow).unwrap().name,
        FEATURE_SMART_ESCROW_NAME
    );
    assert_eq!(
        registered_feature(&lending_v1_2).unwrap().name,
        FEATURE_LENDING_PROTOCOL_V1_2_NAME
    );
    let cleanup = registered_feature(&cleanup_3_5).expect("registered cleanup amendment");
    assert_eq!(cleanup.name, FIX_CLEANUP_3_5_0_NAME);
    assert!(cleanup.supported);
    assert_eq!(cleanup.vote, RegisteredFeatureVote::DefaultNo);
}

#[test]
fn smart_escrow_sfields_have_pinned_wire_definitions() {
    let expected = [
        ("sfGasLimit", SerializedTypeId::UInt32, 81),
        ("sfBytecodeSizeLimit", SerializedTypeId::UInt32, 82),
        ("sfGasPrice", SerializedTypeId::UInt32, 83),
        ("sfGas", SerializedTypeId::UInt32, 84),
        ("sfGasUsed", SerializedTypeId::UInt32, 85),
        ("sfVMReturnCode", SerializedTypeId::Int32, 3),
        ("sfBytecode", SerializedTypeId::VariableLength, 47),
    ];

    for (symbol, type_id, value) in expected {
        let field = get_field_by_symbol(symbol);
        assert_eq!(field.field_type(), type_id, "{symbol} type");
        assert_eq!(field.field_value(), value, "{symbol} field number");
        assert_eq!(
            field.code(),
            field_code(type_id, value),
            "{symbol} wire code"
        );
    }
}

#[test]
fn smart_escrow_ter_catalog_is_pinned() {
    let expected = [
        (
            Ter::TEM_INVALID_BYTECODE,
            -247,
            "temINVALID_BYTECODE",
            "Malformed: Provided byte code is invalid.",
        ),
        (
            Ter::TEM_TEMP_DISABLED,
            -246,
            "temTEMP_DISABLED",
            "The transaction requires logic that is currently temporarily disabled.",
        ),
        (
            Ter::TEF_NO_BYTECODE,
            -175,
            "tefNO_BYTECODE",
            "There is no WASM code to run, but a WASM-specific field was included.",
        ),
        (
            Ter::TEF_BYTECODE_NOT_INCLUDED,
            -174,
            "tefBYTECODE_NOT_INCLUDED",
            "WASM code requires a field that was not included.",
        ),
        (
            Ter::TEC_OUT_OF_GAS,
            201,
            "tecOUT_OF_GAS",
            "The WASM code ran out of gas during execution.",
        ),
        (
            Ter::TEC_BYTECODE_REJECTED,
            202,
            "tecBYTECODE_REJECTED",
            "The custom WASM code that was run rejected your transaction.",
        ),
    ];

    for (code, value, token, human) in expected {
        assert_eq!(code.to_int(), value);
        assert_eq!(trans_token(code), token);
        assert_eq!(trans_human(code), human);
        assert_eq!(trans_code(token), Some(code));
    }
}

#[test]
fn smart_escrow_transaction_and_ledger_formats_are_golden() {
    let tx_formats = TxFormats::get_instance();
    let escrow_create = tx_formats.find_by_type(TxType::ESCROW_CREATE).unwrap();
    let escrow_create_fields = escrow_create
        .so_template()
        .elements()
        .iter()
        .take(8)
        .map(|element| element.sfield().symbol_name())
        .collect::<Vec<_>>();
    assert_eq!(
        escrow_create_fields,
        [
            "sfDestination",
            "sfDestinationTag",
            "sfAmount",
            "sfCondition",
            "sfCancelAfter",
            "sfFinishAfter",
            "sfBytecode",
            "sfData",
        ]
    );
    assert_eq!(
        escrow_create
            .so_template()
            .style(get_field_by_symbol("sfBytecode")),
        Ok(SOEStyle::Optional)
    );

    let escrow_finish = tx_formats.find_by_type(TxType::ESCROW_FINISH).unwrap();
    assert_eq!(
        escrow_finish
            .so_template()
            .style(get_field_by_symbol("sfGas")),
        Ok(SOEStyle::Optional)
    );
    let set_fee = tx_formats.find_by_type(TxType::FEE).unwrap();
    for field in ["sfGasLimit", "sfBytecodeSizeLimit", "sfGasPrice"] {
        assert_eq!(
            set_fee.so_template().style(get_field_by_symbol(field)),
            Ok(SOEStyle::Optional),
            "{field} must be in SetFee"
        );
    }

    let ledger_formats = LedgerFormats::get_instance();
    let escrow = ledger_formats
        .find_by_type(LedgerEntryType::Escrow)
        .unwrap();
    for field in ["sfBytecode", "sfData"] {
        assert_eq!(
            escrow.so_template().style(get_field_by_symbol(field)),
            Ok(SOEStyle::Optional),
            "{field} must be in Escrow"
        );
    }
    let fee_settings = ledger_formats
        .find_by_type(LedgerEntryType::FeeSettings)
        .unwrap();
    for field in ["sfGasLimit", "sfBytecodeSizeLimit", "sfGasPrice"] {
        assert_eq!(
            fee_settings.so_template().style(get_field_by_symbol(field)),
            Ok(SOEStyle::Optional),
            "{field} must be in FeeSettings"
        );
    }
}
