use protocol::{
    KnownFormats, KnownFormatsError, LedgerEntryType, LedgerFormats, SOEStyle, SOETxMPTIssue,
    SOElement, SOTemplate, TxFormats, TxType, get_field_by_symbol,
};

#[test]
fn protocol_known_formats_duplicate_and_unknown_name_errors_match_port_contract() {
    let account = SOElement::new(get_field_by_symbol("sfAccount"), SOEStyle::Required)
        .expect("account field should be useful");
    let sequence = SOElement::new(get_field_by_symbol("sfSequence"), SOEStyle::Optional)
        .expect("sequence field should be useful");
    let template = SOTemplate::new(vec![account], vec![sequence]).expect("template should build");
    let mut formats = KnownFormats::<u16>::new("ParityFormats");

    formats
        .add("First", 1, template.clone(), ())
        .expect("first format should register");

    assert_eq!(
        formats.add("Second", 1, template.clone(), ()),
        Err(KnownFormatsError::DuplicateType {
            registry: "ParityFormats",
            name: "Second",
            existing_name: "First",
        })
    );
    assert_eq!(
        formats.add("First", 2, template.clone(), ()),
        Err(KnownFormatsError::DuplicateName {
            registry: "ParityFormats",
            name: "First",
        })
    );
    assert_eq!(
        formats.find_type_by_name("abcdefghijklmnopqrstuvwxyz0123456789"),
        Err(KnownFormatsError::UnknownFormatName {
            registry: "ParityFormats",
            name: "abcdefghijklmnopqrstuvwxyz012345".to_string(),
        })
    );
}

#[test]
fn protocol_tx_and_ledger_formats_resolve_names_through_shared_registry_shape() {
    let tx_formats = TxFormats::get_instance();
    let ledger_formats = LedgerFormats::get_instance();

    assert_eq!(
        tx_formats
            .find_type_by_name("EnableAmendment")
            .expect("tx name should resolve"),
        TxType::AMENDMENT
    );
    assert_eq!(
        ledger_formats
            .find_type_by_name("AccountRoot")
            .expect("ledger name should resolve"),
        LedgerEntryType::AccountRoot
    );
    assert_eq!(
        tx_formats
            .find_by_type(TxType::DEPOSIT_PREAUTH)
            .expect("tx type should resolve")
            .name(),
        "DepositPreauth"
    );
    assert_eq!(
        ledger_formats
            .find_by_type(LedgerEntryType::DepositPreauth)
            .expect("ledger type should resolve")
            .metadata()
            .rpc_name,
        "deposit_preauth"
    );
    assert_eq!(tx_formats.iter().count(), 75);
    assert_eq!(tx_formats.get_common_fields().len(), 17);
    assert_eq!(
        tx_formats
            .find_by_type(TxType::PAYMENT)
            .expect("payment type should resolve")
            .so_template()
            .size(),
        26
    );
    let amount_index = tx_formats
        .find_by_type(TxType::PAYMENT)
        .expect("payment type should resolve")
        .so_template()
        .get_index(get_field_by_symbol("sfAmount"))
        .expect("amount field index should resolve");
    assert_eq!(
        tx_formats
            .find_by_type(TxType::PAYMENT)
            .expect("payment type should resolve")
            .so_template()
            .elements()[amount_index as usize]
            .support_mpt(),
        SOETxMPTIssue::Supported
    );
}
