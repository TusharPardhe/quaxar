use protocol::{
    LedgerEntryType, LedgerFormats, SOETxMPTIssue, TxFormats, TxType, get_field_by_symbol,
};

#[test]
fn tx_formats_registry_matches_current_cpp_catalog_shape() {
    let formats = TxFormats::get_instance();
    let common_symbols = formats
        .get_common_fields()
        .iter()
        .map(|element| element.sfield().symbol_name())
        .collect::<Vec<_>>();
    let registry_names = formats.iter().map(|item| item.name()).collect::<Vec<_>>();

    assert_eq!(formats.iter().count(), 75);
    assert_eq!(formats.get_common_fields().len(), 17);
    assert_eq!(
        common_symbols,
        vec![
            "sfTransactionType",
            "sfFlags",
            "sfSourceTag",
            "sfAccount",
            "sfSequence",
            "sfPreviousTxnID",
            "sfLastLedgerSequence",
            "sfAccountTxnID",
            "sfFee",
            "sfOperationLimit",
            "sfMemos",
            "sfSigningPubKey",
            "sfTicketSequence",
            "sfTxnSignature",
            "sfSigners",
            "sfNetworkID",
            "sfDelegate",
        ]
    );
    assert_eq!(registry_names.first(), Some(&"Payment"));
    assert_eq!(registry_names.last(), Some(&"UNLModify"));
    assert_eq!(
        formats
            .find_type_by_name("EnableAmendment")
            .expect("amendment type"),
        TxType::AMENDMENT
    );
    assert!(formats.find_by_type(TxType::HOOK_SET).is_none());

    let payment = formats
        .find_by_type(TxType::PAYMENT)
        .expect("payment format");
    assert_eq!(payment.name(), "Payment");
    assert_eq!(payment.metadata().tag_name, "ttPAYMENT");
    assert_eq!(payment.metadata().privileges, "createAcct");
    assert_eq!(payment.so_template().size(), 26);

    let amount_index = payment
        .so_template()
        .get_index(get_field_by_symbol("sfAmount"))
        .expect("amount field index");
    assert_eq!(
        payment.so_template().elements()[amount_index as usize].support_mpt(),
        SOETxMPTIssue::Supported
    );
    assert_eq!(
        formats
            .find_by_type(TxType::UNL_MODIFY)
            .expect("unl modify format")
            .name(),
        "UNLModify"
    );
}

#[test]
fn ledger_formats_registry_preserves_name_and_rpc_name_independently() {
    let ledger_formats = LedgerFormats::get_instance();
    let tx_formats = TxFormats::get_instance();

    let account_root = ledger_formats
        .find_by_type(LedgerEntryType::AccountRoot)
        .expect("account root format");
    assert_eq!(account_root.name(), "AccountRoot");
    assert_eq!(account_root.metadata().rpc_name, "account");

    let deposit_ledger = ledger_formats
        .find_by_name("DepositPreauth")
        .expect("ledger deposit preauth");
    let deposit_tx = tx_formats
        .find_by_name("DepositPreauth")
        .expect("tx deposit preauth");

    assert_eq!(
        deposit_ledger.format_type(),
        LedgerEntryType::DepositPreauth
    );
    assert_eq!(deposit_ledger.metadata().rpc_name, "deposit_preauth");
    assert_eq!(deposit_tx.format_type(), TxType::DEPOSIT_PREAUTH);
}
