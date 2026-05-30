use std::collections::BTreeMap;
use std::sync::Arc;

use protocol::{AccountID, JsonValue, STTx, to_base58, trans_token};
use tx::TxDetails;

use crate::ledger_to_json::ledger_to_json_tx::fill_json_queue_tx;
use crate::{AppLedgerFill, copy_from};

pub(crate) fn fill_json_queue(
    json: &mut JsonValue,
    fill: &AppLedgerFill<'_>,
    tx_queue: &[TxDetails<Arc<STTx>, AccountID>],
) {
    let JsonValue::Object(root) = json else {
        panic!("ledger json root must be an object");
    };

    let queue = tx_queue
        .iter()
        .map(|tx| {
            let mut tx_json = BTreeMap::new();
            tx_json.insert(
                "fee_level".to_owned(),
                JsonValue::String(tx.fee_level.to_string()),
            );
            if let Some(last_valid) = tx.last_valid {
                tx_json.insert(
                    "LastLedgerSequence".to_owned(),
                    JsonValue::Unsigned(u64::from(last_valid)),
                );
            }

            tx_json.insert(
                "fee".to_owned(),
                JsonValue::String(tx.consequences.fee().to_string()),
            );
            let spend = tx
                .consequences
                .potential_spend()
                .saturating_add(tx.consequences.fee());
            tx_json.insert(
                "max_spend_drops".to_owned(),
                JsonValue::String(spend.to_string()),
            );
            tx_json.insert(
                "auth_change".to_owned(),
                JsonValue::Bool(tx.consequences.is_blocker()),
            );
            tx_json.insert(
                "account".to_owned(),
                JsonValue::String(to_base58(tx.account)),
            );
            tx_json.insert(
                "retries_remaining".to_owned(),
                JsonValue::Signed(i64::from(tx.retries_remaining)),
            );
            tx_json.insert(
                "preflight_result".to_owned(),
                JsonValue::String(trans_token(tx.preflight_result).to_owned()),
            );
            if let Some(last_result) = tx.last_result {
                tx_json.insert(
                    "last_result".to_owned(),
                    JsonValue::String(trans_token(last_result).to_owned()),
                );
            }

            let temp = fill_json_queue_tx(fill, tx.tx.as_ref());
            let mut entry = JsonValue::Object(tx_json);
            match temp {
                JsonValue::Object(_) => {
                    if fill.api_version() > 1 {
                        copy_from(&mut entry, &temp);
                    } else {
                        let JsonValue::Object(object) = &mut entry else {
                            unreachable!("queue entry shell should be an object");
                        };
                        let nested = object.entry("tx".to_owned()).or_insert(JsonValue::Null);
                        copy_from(nested, &temp);
                    }
                }
                other => {
                    let JsonValue::Object(object) = &mut entry else {
                        unreachable!("queue entry shell should be an object");
                    };
                    if fill.api_version() > 1 {
                        object.insert("hash".to_owned(), other);
                    } else {
                        object.insert("tx".to_owned(), other);
                    }
                }
            }

            entry
        })
        .collect();

    root.insert("queue_data".to_owned(), JsonValue::Array(queue));
}
