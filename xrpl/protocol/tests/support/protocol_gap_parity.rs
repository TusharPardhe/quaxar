use basics::{
    buffer::Buffer, number::NumberParts as RuntimeNumber, string_utilities::sql_blob_literal,
};
use protocol::{
    Asset, AssetAmountType, AssetToken, Directory, Domain, Issue, LedgerHash, STAmount, STObject,
    STTx, StBase, TxType, TxnSql, bad_asset, currency_from_string, exchange_erase, exchange_get,
    exchange_set, exchange_set_blob_with, get_field_by_symbol, make_mpt_id, xrp_issue,
};

fn account(fill: u8) -> protocol::AccountID {
    protocol::AccountID::from_array([fill; 20])
}

fn payment_tx(sequence: u32) -> STTx {
    STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(0x11));
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(0x22));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    })
}

#[test]
fn asset_helpers_cover_cpp_variant_contract() {
    let usd = currency_from_string("USD");
    let issue = Issue::new(usd, account(0x44));
    let asset = Asset::from(issue);

    assert!(asset.holds::<Issue>());
    assert!(!asset.holds::<protocol::MPTIssue>());
    assert_eq!(*asset.get::<Issue>(), issue);
    assert_eq!(asset.token(), AssetToken::Currency(usd));
    assert_eq!(asset.get_amount_type(), AssetAmountType::IOU);
    assert_eq!(asset, usd);
    assert_ne!(asset, bad_asset());

    let amount = asset
        .amount(
            RuntimeNumber::try_from_external_parts(12345, -2, basics::number::get_mantissa_scale())
                .expect("number"),
        )
        .expect("asset amount");
    assert_eq!(amount.asset(), asset);
    assert_eq!(amount.iou().to_string(), "123.45");

    let bad_mpt = Asset::from(protocol::MPTIssue::new(make_mpt_id(
        7,
        protocol::AccountID::zero(),
    )));
    assert_eq!(bad_mpt, bad_asset());
    assert_eq!(
        Asset::from(xrp_issue()).get_amount_type(),
        AssetAmountType::XRP
    );
}

#[test]
fn st_exchange_helpers_round_trip_typed_fields() {
    let mut object = STObject::new(get_field_by_symbol("sfGeneric"));
    let memo_type = get_field_by_symbol("sfMemoType");
    let sequence = get_field_by_symbol("sfSequence");

    exchange_set(&mut object, sequence, 99u32);
    assert_eq!(exchange_get::<u32>(&object, sequence), Some(99));

    exchange_set_blob_with(&mut object, memo_type, 4, |bytes| {
        bytes.copy_from_slice(b"TEST")
    });
    assert_eq!(
        exchange_get::<Buffer>(&object, memo_type),
        Some(Buffer::from_bytes(b"TEST"))
    );

    exchange_erase(&mut object, memo_type);
    assert_eq!(exchange_get::<Vec<u8>>(&object, memo_type), None);
}

#[test]
fn sttx_meta_sql_helpers_match_cpp_string_shape() {
    let tx = payment_tx(7);
    let mut raw_txn = protocol::Serializer::default();
    tx.add(&mut raw_txn);

    let escaped_meta = "X'ABCD'";
    let expected = format!(
        "('{}', '{}', '{}', '{}', '{}', '{}', {}, {})",
        tx.get_transaction_id(),
        "Payment",
        protocol::to_base58(account(0x11)),
        7,
        1234,
        TxnSql::Included.as_char(),
        sql_blob_literal(raw_txn.peek_data()),
        escaped_meta
    );

    assert_eq!(
        STTx::get_meta_sql_insert_replace_header(),
        "INSERT OR REPLACE INTO Transactions (TransID, TransType, FromAcct, FromSeq, LedgerSeq, Status, RawTxn, TxnMeta) VALUES "
    );
    assert_eq!(
        tx.get_meta_sql_with_raw_txn(raw_txn.clone(), 1234, TxnSql::Included, escaped_meta),
        expected
    );
    assert_eq!(
        tx.get_meta_sql(1234, escaped_meta),
        tx.get_meta_sql_with_raw_txn(raw_txn, 1234, TxnSql::Validated, escaped_meta)
    );
}

#[test]
fn uint_type_aliases_cover_cpp_protocol_surface() {
    assert_eq!(Directory::BYTES, 32);
    assert_eq!(Domain::BYTES, 32);
    assert_eq!(LedgerHash::BYTES, 32);
}
