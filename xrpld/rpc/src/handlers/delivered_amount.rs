//! Delivered amount utility port from `xrpld/rpc/DeliveredAmount.*`.

use protocol::{
    JsonOptions, JsonValue, STAmount, STTx, StBase, TxMeta, TxType, get_field_by_symbol,
    is_tes_success,
};

pub const DELIVERED_AMOUNT_SWITCH_LEDGER: u32 = 4_594_095;
pub const DELIVERED_AMOUNT_SWITCH_CLOSE_TIME: u32 = 446_000_000;

pub fn can_have_delivered_amount(txn: &STTx, meta: &TxMeta) -> bool {
    matches!(
        txn.get_txn_type(),
        TxType::PAYMENT | TxType::CHECK_CASH | TxType::ACCOUNT_DELETE
    ) && is_tes_success(meta.get_result_ter())
}

pub fn get_delivered_amount(
    ledger_seq: u32,
    close_time: Option<u32>,
    txn: Option<&STTx>,
    meta: &TxMeta,
) -> Option<STAmount> {
    let txn = txn?;

    if let Some(amount) = meta.get_delivered_amount() {
        return Some(amount.clone());
    }

    if !txn.is_field_present(get_field_by_symbol("sfAmount")) {
        return None;
    }

    if ledger_seq >= DELIVERED_AMOUNT_SWITCH_LEDGER
        || close_time.is_some_and(|value| value > DELIVERED_AMOUNT_SWITCH_CLOSE_TIME)
    {
        return Some(txn.get_field_amount(get_field_by_symbol("sfAmount")));
    }

    None
}

pub fn insert_delivered_amount(
    meta: &mut JsonValue,
    ledger_seq: u32,
    close_time: Option<u32>,
    txn: &STTx,
    transaction_meta: &TxMeta,
) {
    if !can_have_delivered_amount(txn, transaction_meta) {
        return;
    }

    let delivered = get_delivered_amount(ledger_seq, close_time, Some(txn), transaction_meta)
        .map(|amount| amount.json(JsonOptions::INCLUDE_DATE))
        .unwrap_or_else(|| JsonValue::String("unavailable".to_owned()));

    let JsonValue::Object(object) = meta else {
        return;
    };
    object.insert("delivered_amount".to_owned(), delivered);
}

#[cfg(test)]
mod tests {
    use super::{
        DELIVERED_AMOUNT_SWITCH_CLOSE_TIME, DELIVERED_AMOUNT_SWITCH_LEDGER,
        can_have_delivered_amount, get_delivered_amount, insert_delivered_amount,
    };
    use basics::base_uint::Uint256;
    use protocol::{
        AccountID, JsonValue, STAmount, STArray, STObject, STTx, TxMeta, TxType,
        get_field_by_symbol,
    };

    fn account(fill: u8) -> AccountID {
        AccountID::from_array([fill; 20])
    }

    fn payment_tx() -> STTx {
        STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account(1));
            tx.set_account_id(get_field_by_symbol("sfDestination"), account(2));
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(1_000_000, false),
            );
        })
    }

    fn payment_meta(tx_id: Uint256, ledger_seq: u32, delivered: Option<STAmount>) -> TxMeta {
        let mut object = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
        object.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
        object.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 2);
        object.set_field_array(
            get_field_by_symbol("sfAffectedNodes"),
            STArray::new(get_field_by_symbol("sfAffectedNodes")),
        );
        if let Some(delivered) = delivered {
            object.set_field_amount(get_field_by_symbol("sfDeliveredAmount"), delivered);
        }
        TxMeta::from_stobject(tx_id, ledger_seq, object)
    }

    #[test]
    fn delivered_amount_prefers_meta_then_falls_back() {
        let tx = payment_tx();
        let tx_id = tx.get_transaction_id();
        let meta = payment_meta(tx_id, DELIVERED_AMOUNT_SWITCH_LEDGER, None);
        assert!(can_have_delivered_amount(&tx, &meta));
        assert_eq!(
            get_delivered_amount(DELIVERED_AMOUNT_SWITCH_LEDGER, None, Some(&tx), &meta),
            Some(STAmount::new_native(1_000_000, false))
        );

        let meta_with_delivered = payment_meta(tx_id, 1, Some(STAmount::new_native(22, false)));
        assert_eq!(
            get_delivered_amount(1, Some(0), Some(&tx), &meta_with_delivered),
            Some(STAmount::new_native(22, false))
        );

        let meta_without_delivered = payment_meta(tx_id, 1, None);
        assert_eq!(
            get_delivered_amount(
                1,
                Some(DELIVERED_AMOUNT_SWITCH_CLOSE_TIME + 1),
                Some(&tx),
                &meta_without_delivered
            ),
            Some(STAmount::new_native(1_000_000, false))
        );
    }

    #[test]
    fn delivered_amount_inserts_unavailable_only_for_eligible_transactions() {
        let tx = payment_tx();
        let tx_id = tx.get_transaction_id();
        let meta = payment_meta(tx_id, 1, None);
        let mut json = JsonValue::Object(std::collections::BTreeMap::new());
        insert_delivered_amount(&mut json, 1, None, &tx, &meta);
        let JsonValue::Object(object) = json else {
            panic!("json must be an object");
        };
        assert_eq!(
            object.get("delivered_amount"),
            Some(&JsonValue::String("unavailable".to_owned()))
        );

        let mut account_delete = payment_tx();
        account_delete.set_field_u16(
            get_field_by_symbol("sfTransactionType"),
            u16::from(TxType::ACCOUNT_DELETE),
        );
        let mut json = JsonValue::Object(std::collections::BTreeMap::new());
        insert_delivered_amount(&mut json, 1, None, &account_delete, &meta);
        let JsonValue::Object(object) = json else {
            panic!("json must be an object");
        };
        assert!(object.contains_key("delivered_amount"));
    }

    #[test]
    fn delivered_amount_skips_ineligible_transactions() {
        let tx = STTx::new(TxType::OFFER_CREATE, |_| {});
        let meta = payment_meta(tx.get_transaction_id(), 1, None);
        let mut json = JsonValue::Object(std::collections::BTreeMap::new());
        insert_delivered_amount(&mut json, 1, None, &tx, &meta);
        let JsonValue::Object(object) = json else {
            panic!("json must be an object");
        };
        assert!(!object.contains_key("delivered_amount"));
    }
}
