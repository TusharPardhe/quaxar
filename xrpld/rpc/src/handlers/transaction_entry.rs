//! Transaction-entry RPC handler slice.

use basics::{
    base_uint::Uint256,
    chrono::{NetClockTimePoint, to_string_iso},
};
use protocol::{JsonOptions, JsonValue, STTx, StBase, TxMeta};

use crate::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcRole, insert_deliver_max,
    lookup_ledger_with_result,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionEntryRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait TransactionEntrySource: LedgerLookupSource {
    fn read_transaction_entry(
        &self,
        ledger: &LedgerLookupLedger,
        tx_hash: Uint256,
    ) -> Option<(STTx, Option<TxMeta>)>;

    fn get_close_time_by_seq(&self, ledger_seq: u32) -> Option<NetClockTimePoint>;
    fn get_hash_by_seq(&self, ledger_seq: u32) -> Option<Uint256>;
}

fn insert_named_error(result: &mut JsonValue, error: &str) {
    let JsonValue::Object(object) = result else {
        return;
    };
    object.insert("error".to_owned(), JsonValue::String(error.to_owned()));
}

fn parse_tx_hash(params: &JsonValue) -> Result<Uint256, &'static str> {
    let JsonValue::Object(object) = params else {
        return Err("fieldNotFoundTransaction");
    };

    let Some(tx_hash) = object.get("tx_hash") else {
        return Err("fieldNotFoundTransaction");
    };

    let JsonValue::String(tx_hash) = tx_hash else {
        return Err("malformedRequest");
    };

    Uint256::from_hex(tx_hash).map_err(|_| "malformedRequest")
}

fn has_tx_hash(params: &JsonValue) -> bool {
    matches!(params, JsonValue::Object(object) if object.contains_key("tx_hash"))
}

pub fn do_transaction_entry<S: TransactionEntrySource>(
    request: &TransactionEntryRequest<'_>,
    source: &S,
) -> JsonValue {
    tracing::trace!(target: "rpc", method = "transaction_entry", "transaction_entry query");
    let context = LedgerLookupContext {
        params: request.params,
        source,
        api_version: request.api_version,
        role: request.role,
    };

    let (ledger, mut result) = match lookup_ledger_with_result(&context) {
        Ok(result) => result,
        Err(status) => {
            let mut error = JsonValue::Object(std::collections::BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
    };

    if !has_tx_hash(request.params) {
        insert_named_error(&mut result, "fieldNotFoundTransaction");
        return result;
    }

    let JsonValue::Object(object) = &result else {
        return result;
    };
    if !object.contains_key("ledger_hash") {
        let mut result = result;
        insert_named_error(&mut result, "notYetImplemented");
        return result;
    }

    let tx_hash = match parse_tx_hash(request.params) {
        Ok(hash) => hash,
        Err(error) => {
            insert_named_error(&mut result, error);
            return result;
        }
    };

    let Some((txn, meta)) = source.read_transaction_entry(&ledger, tx_hash) else {
        insert_named_error(&mut result, "transactionNotFound");
        return result;
    };

    let JsonValue::Object(object) = &mut result else {
        return result;
    };
    let mut tx_json = if request.api_version > 1 {
        txn.json(JsonOptions::DISABLE_API_PRIOR_V2)
    } else {
        txn.json(JsonOptions::NONE)
    };
    insert_deliver_max(&mut tx_json, txn.get_txn_type(), request.api_version);
    object.insert("tx_json".to_owned(), tx_json);

    if request.api_version > 1 {
        object.insert(
            "hash".to_owned(),
            JsonValue::String(txn.get_transaction_id().to_string()),
        );
        if !ledger.open
            && let Some(ledger_hash) = source.get_hash_by_seq(ledger.seq)
        {
            object.insert(
                "ledger_hash".to_owned(),
                JsonValue::String(ledger_hash.to_string()),
            );
        }
        let validated = source.is_validated(&ledger);
        object.insert("validated".to_owned(), JsonValue::Bool(validated));
        if validated {
            object.insert(
                "ledger_index".to_owned(),
                JsonValue::Unsigned(u64::from(ledger.seq)),
            );
            if let Some(close_time) = source.get_close_time_by_seq(ledger.seq) {
                object.insert(
                    "close_time_iso".to_owned(),
                    JsonValue::String(to_string_iso(close_time)),
                );
            }
        }
    }

    if let Some(meta) = meta {
        let key = if request.api_version > 1 {
            "meta"
        } else {
            "metadata"
        };
        object.insert(key.to_owned(), meta.get_json(JsonOptions::NONE));
    }

    result
}
