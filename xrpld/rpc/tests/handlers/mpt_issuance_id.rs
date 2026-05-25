//! Tests for the mpt issuance id RPC handler.

use basics::base_uint::Uint256;
use protocol::{
    AccountID, JsonValue, LedgerEntryType, STAmount, STArray, STObject, STTx, TxMeta, TxType,
    get_field_by_symbol, make_mpt_id,
};
use rpc::{get_id_from_created_issuance, insert_mp_token_issuance_id};

fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn issuance_tx(issuer: AccountID) -> STTx {
    STTx::new(TxType::MPTOKEN_ISSUANCE_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), issuer);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 7);
    })
}

fn issuance_meta(tx_id: Uint256, ledger_seq: u32, sequence: u32, issuer: AccountID) -> TxMeta {
    let mut new_fields = STObject::new(get_field_by_symbol("sfNewFields"));
    new_fields.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    new_fields.set_account_id(get_field_by_symbol("sfIssuer"), issuer);

    let mut created = STObject::new(get_field_by_symbol("sfCreatedNode"));
    created.set_field_h256(
        get_field_by_symbol("sfLedgerIndex"),
        Uint256::from_array([0x55; 32]),
    );
    created.set_field_u16(
        get_field_by_symbol("sfLedgerEntryType"),
        LedgerEntryType::MPTokenIssuance.code(),
    );
    created.set_field_object(get_field_by_symbol("sfNewFields"), new_fields);

    let mut affected = STArray::new(get_field_by_symbol("sfAffectedNodes"));
    affected.push_back(created);

    let mut object = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    object.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    object.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 3);
    object.set_field_array(get_field_by_symbol("sfAffectedNodes"), affected);
    TxMeta::from_stobject(tx_id, ledger_seq, object)
}

#[test]
fn mpt_issuance_id_created_node_parsing() {
    let issuer = account(0x33);
    let tx = issuance_tx(issuer);
    let meta = issuance_meta(tx.get_transaction_id(), 1, 9, issuer);

    assert_eq!(
        get_id_from_created_issuance(&meta),
        Some(make_mpt_id(9, issuer))
    );

    let mut json = JsonValue::Object(std::collections::BTreeMap::new());
    insert_mp_token_issuance_id(&mut json, &tx, &meta);
    let JsonValue::Object(object) = json else {
        panic!("json should be an object");
    };
    assert_eq!(
        object.get("mpt_issuance_id"),
        Some(&JsonValue::String(make_mpt_id(9, issuer).to_string()))
    );
}

#[test]
fn mpt_issuance_id_skips_missing_created_node() {
    let issuer = account(0x33);
    let tx = issuance_tx(issuer);

    let mut object = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    object.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    object.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 3);
    object.set_field_array(
        get_field_by_symbol("sfAffectedNodes"),
        STArray::new(get_field_by_symbol("sfAffectedNodes")),
    );
    let meta = TxMeta::from_stobject(tx.get_transaction_id(), 1, object);

    let mut json = JsonValue::Object(std::collections::BTreeMap::new());
    insert_mp_token_issuance_id(&mut json, &tx, &meta);

    let JsonValue::Object(object) = json else {
        panic!("json should be an object");
    };

    assert!(!object.contains_key("mpt_issuance_id"));
}
