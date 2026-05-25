//! MPToken issuance ID helpers ported from `xrpld/rpc/MPTokenIssuanceID.*`.

use protocol::{
    JsonValue, LedgerEntryType, MPTID, STTx, StBase, TxMeta, TxType, get_field_by_symbol,
    is_tes_success, make_mpt_id,
};

pub fn can_have_mp_token_issuance_id(txn: &STTx, transaction_meta: &TxMeta) -> bool {
    txn.get_txn_type() == TxType::MPTOKEN_ISSUANCE_CREATE
        && is_tes_success(transaction_meta.get_result_ter())
}

pub fn get_id_from_created_issuance(transaction_meta: &TxMeta) -> Option<MPTID> {
    for node in transaction_meta.get_nodes().iter() {
        if node.get_field_u16(get_field_by_symbol("sfLedgerEntryType"))
            != LedgerEntryType::MPTokenIssuance.code()
            || node.fname() != get_field_by_symbol("sfCreatedNode")
        {
            continue;
        }

        let issuance = node.get_field_object(get_field_by_symbol("sfNewFields"));
        return Some(make_mpt_id(
            issuance.get_field_u32(get_field_by_symbol("sfSequence")),
            issuance.get_account_id(get_field_by_symbol("sfIssuer")),
        ));
    }

    None
}

pub fn insert_mp_token_issuance_id(
    response: &mut JsonValue,
    transaction: &STTx,
    transaction_meta: &TxMeta,
) {
    if !can_have_mp_token_issuance_id(transaction, transaction_meta) {
        return;
    }

    let Some(result) = get_id_from_created_issuance(transaction_meta) else {
        return;
    };

    let JsonValue::Object(object) = response else {
        return;
    };
    object.insert(
        "mpt_issuance_id".to_owned(),
        JsonValue::String(result.to_string()),
    );
}

#[cfg(test)]
mod tests {
    use super::{
        can_have_mp_token_issuance_id, get_id_from_created_issuance, insert_mp_token_issuance_id,
    };
    use basics::base_uint::Uint256;
    use protocol::{
        AccountID, JsonValue, LedgerEntryType, STArray, STObject, STTx, TxMeta, TxType,
        get_field_by_symbol, make_mpt_id,
    };

    fn account(fill: u8) -> AccountID {
        AccountID::from_array([fill; 20])
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

    fn issuance_tx(issuer: AccountID) -> STTx {
        STTx::new(TxType::MPTOKEN_ISSUANCE_CREATE, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), issuer);
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                protocol::STAmount::new_native(10, false),
            );
            tx.set_field_u32(get_field_by_symbol("sfSequence"), 7);
        })
    }

    #[test]
    fn mpt_issuance_id_round_trips() {
        let issuer = account(0x33);
        let tx = issuance_tx(issuer);
        let meta = issuance_meta(tx.get_transaction_id(), 1, 9, issuer);

        assert!(can_have_mp_token_issuance_id(&tx, &meta));
        assert_eq!(
            get_id_from_created_issuance(&meta),
            Some(make_mpt_id(9, issuer))
        );

        let mut json = JsonValue::Object(std::collections::BTreeMap::new());
        insert_mp_token_issuance_id(&mut json, &tx, &meta);
        let JsonValue::Object(object) = json else {
            panic!("json must be an object");
        };
        assert_eq!(
            object.get("mpt_issuance_id"),
            Some(&JsonValue::String(make_mpt_id(9, issuer).to_string()))
        );
    }
}
