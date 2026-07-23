//! Tests for the delivered amount RPC handler.

use basics::base_uint::Uint256;
use protocol::{
    AccountID, JsonOptions, JsonValue, MPTAmount, MPTIssue, STAmount, STArray, STObject, STTx,
    StBase, TxMeta, TxType, get_field_by_symbol, make_mpt_id,
};
use rpc::{DELIVERED_AMOUNT_SWITCH_LEDGER, get_delivered_amount, insert_delivered_amount};

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
fn fix_mpt_delivered_amount_rpc_uses_canonical_metadata_and_preserves_unavailable_off() {
    let mpt_issue = MPTIssue::new(make_mpt_id(1, account(3)));
    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(1));
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(2));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::from_mpt_amount(
                get_field_by_symbol("sfAmount"),
                MPTAmount::from_value(1_000),
                mpt_issue,
            ),
        );
    });
    let delivered = STAmount::from_mpt_amount(
        get_field_by_symbol("sfDeliveredAmount"),
        MPTAmount::from_value(800),
        mpt_issue,
    );
    let tx_id = tx.get_transaction_id();

    let on_meta = payment_meta(tx_id, 1, Some(delivered));
    let mut on_json = on_meta.get_json(JsonOptions::INCLUDE_DATE);
    insert_delivered_amount(&mut on_json, 1, None, &tx, &on_meta);
    let JsonValue::Object(on_json) = on_json else {
        panic!("metadata JSON should be an object");
    };
    assert_eq!(
        on_json.get("DeliveredAmount"),
        on_json.get("delivered_amount")
    );

    let off_meta = payment_meta(tx_id, 1, None);
    let mut off_json = off_meta.get_json(JsonOptions::INCLUDE_DATE);
    insert_delivered_amount(&mut off_json, 1, None, &tx, &off_meta);
    let JsonValue::Object(off_json) = off_json else {
        panic!("metadata JSON should be an object");
    };
    assert!(!off_json.contains_key("DeliveredAmount"));
    assert_eq!(
        off_json.get("delivered_amount"),
        Some(&JsonValue::String("unavailable".to_owned()))
    );
}

#[test]
fn delivered_amount_prefers_meta_then_amount() {
    let tx = payment_tx();
    let tx_id = tx.get_transaction_id();
    let meta = payment_meta(tx_id, DELIVERED_AMOUNT_SWITCH_LEDGER, None);
    assert_eq!(
        get_delivered_amount(DELIVERED_AMOUNT_SWITCH_LEDGER, None, Some(&tx), &meta),
        Some(STAmount::new_native(1_000_000, false))
    );

    let meta = payment_meta(tx_id, 1, Some(STAmount::new_native(22, false)));
    assert_eq!(
        get_delivered_amount(1, None, Some(&tx), &meta),
        Some(STAmount::new_native(22, false))
    );

    let meta_without_delivered = payment_meta(tx_id, 1, None);
    assert_eq!(
        get_delivered_amount(
            1,
            Some(rpc::DELIVERED_AMOUNT_SWITCH_CLOSE_TIME + 1),
            Some(&tx),
            &meta_without_delivered
        ),
        Some(STAmount::new_native(1_000_000, false))
    );
}

#[test]
fn delivered_amount_inserts_unavailable_for_eligible_missing_amount() {
    let tx = payment_tx();
    let tx_id = tx.get_transaction_id();
    let meta = payment_meta(tx_id, 1, None);
    let mut json = JsonValue::Object(std::collections::BTreeMap::new());
    insert_delivered_amount(&mut json, 1, None, &tx, &meta);

    let JsonValue::Object(object) = json else {
        panic!("json should be an object");
    };

    assert_eq!(
        object.get("delivered_amount"),
        Some(&JsonValue::String("unavailable".to_owned()))
    );
}

#[test]
fn delivered_amount_skips_ineligible_transactions() {
    let tx = STTx::new(TxType::OFFER_CREATE, |_| {});
    let meta = payment_meta(tx.get_transaction_id(), 1, None);
    let mut json = JsonValue::Object(std::collections::BTreeMap::new());
    insert_delivered_amount(&mut json, 1, None, &tx, &meta);

    let JsonValue::Object(object) = json else {
        panic!("json should be an object");
    };

    assert!(!object.contains_key("delivered_amount"));
}

#[test]
fn delivered_amount_uses_meta_delivered_amount_when_present() {
    let tx = payment_tx();
    let tx_id = tx.get_transaction_id();
    let delivered = STAmount::new_native(500_000, false);
    let meta = payment_meta(
        tx_id,
        DELIVERED_AMOUNT_SWITCH_LEDGER + 1,
        Some(delivered.clone()),
    );

    let result = get_delivered_amount(
        DELIVERED_AMOUNT_SWITCH_LEDGER + 1,
        Some(rpc::DELIVERED_AMOUNT_SWITCH_CLOSE_TIME + 1),
        Some(&tx),
        &meta,
    );
    assert_eq!(result, Some(delivered));
}

#[test]
fn delivered_amount_falls_back_to_amount_for_old_ledgers() {
    let tx = payment_tx();
    let tx_id = tx.get_transaction_id();
    let meta = payment_meta(tx_id, DELIVERED_AMOUNT_SWITCH_LEDGER, None);

    // At the switch ledger, should use Amount field
    let result = get_delivered_amount(DELIVERED_AMOUNT_SWITCH_LEDGER, None, Some(&tx), &meta);
    assert_eq!(result, Some(STAmount::new_native(1_000_000, false)));
}

#[test]
fn delivered_amount_insert_adds_both_fields_when_present() {
    let tx = payment_tx();
    let tx_id = tx.get_transaction_id();
    let delivered = STAmount::new_native(750_000, false);
    let meta = payment_meta(tx_id, DELIVERED_AMOUNT_SWITCH_LEDGER + 1, Some(delivered));

    let mut json = JsonValue::Object(std::collections::BTreeMap::new());
    insert_delivered_amount(
        &mut json,
        DELIVERED_AMOUNT_SWITCH_LEDGER + 1,
        Some(rpc::DELIVERED_AMOUNT_SWITCH_CLOSE_TIME + 1),
        &tx,
        &meta,
    );

    let JsonValue::Object(object) = json else {
        panic!("json should be an object");
    };
    // Should have delivered_amount field
    assert!(object.contains_key("delivered_amount"));
    let JsonValue::String(amount_str) = object.get("delivered_amount").unwrap() else {
        panic!("delivered_amount should be a string");
    };
    assert_eq!(amount_str, "750000");
}

#[test]
fn delivered_amount_check_cancel_not_eligible() {
    let tx = STTx::new(TxType::CHECK_CANCEL, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(1));
    });
    let meta = payment_meta(tx.get_transaction_id(), 1, None);

    let result = get_delivered_amount(1, None, Some(&tx), &meta);
    assert_eq!(result, None);
}
