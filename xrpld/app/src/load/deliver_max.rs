//! Shared `DeliverMax` JSON shaping ported from the reference implementation.

use protocol::{JsonValue, TxType};

pub fn insert_deliver_max(tx_json: &mut JsonValue, txn_type: TxType, api_version: u32) {
    if txn_type != TxType::PAYMENT {
        return;
    }

    let JsonValue::Object(object) = tx_json else {
        return;
    };

    let Some(amount) = object.get("Amount").cloned() else {
        return;
    };

    object.insert("DeliverMax".to_owned(), amount);
    if api_version > 1 {
        object.remove("Amount");
    }
}

#[cfg(test)]
mod tests {
    use super::insert_deliver_max;
    use std::collections::BTreeMap;

    use protocol::{JsonValue, STAmount, STTx, StBase, TxType, get_field_by_symbol};

    fn payment_json() -> JsonValue {
        STTx::new(TxType::PAYMENT, |tx| {
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(123, false),
            );
        })
        .json(protocol::JsonOptions::NONE)
    }

    #[test]
    fn deliver_max_copies_amount_for_payment_v1() {
        let mut json = payment_json();
        insert_deliver_max(&mut json, TxType::PAYMENT, 1);

        let JsonValue::Object(object) = json else {
            panic!("json must be an object");
        };
        assert_eq!(object.get("DeliverMax"), object.get("Amount"));
    }

    #[test]
    fn deliver_max_removes_amount_for_payment_v2() {
        let mut json = payment_json();
        insert_deliver_max(&mut json, TxType::PAYMENT, 2);

        let JsonValue::Object(object) = json else {
            panic!("json must be an object");
        };
        assert!(object.contains_key("DeliverMax"));
        assert!(!object.contains_key("Amount"));
    }

    #[test]
    fn deliver_max_skips_non_payment_and_missing_amount() {
        let mut offer = payment_json();
        insert_deliver_max(&mut offer, TxType::OFFER_CREATE, 2);
        let JsonValue::Object(offer) = offer else {
            panic!("offer json must be an object");
        };
        assert!(!offer.contains_key("DeliverMax"));
        assert!(offer.contains_key("Amount"));

        let mut no_amount = JsonValue::Object(BTreeMap::new());
        insert_deliver_max(&mut no_amount, TxType::PAYMENT, 2);
        let JsonValue::Object(no_amount) = no_amount else {
            panic!("json must be an object");
        };
        assert!(no_amount.is_empty());
    }
}
