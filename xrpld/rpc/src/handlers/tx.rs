//! Transaction RPC handler slice.

use std::{collections::BTreeMap, sync::Arc};

use app::{TransStatus, Transaction};
use basics::{base_uint::Uint256, chrono::to_string_iso, str_hex::str_hex};
use protocol::{JsonOptions, JsonValue, STTx};

use crate::{
    RpcErrorCode, TxLookupError, TxLookupOutcome, TxRecord, decode_ctid, encode_ctid,
    insert_deliver_max, insert_delivered_amount, insert_mp_token_issuance_id,
    insert_nft_synthetic_in_json, make_error_message, rpc_error,
};
use protocol::TxSearched;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
}

pub trait TxSource {
    fn tx_tables_enabled(&self) -> bool;
    fn network_id(&self) -> u32;
    fn network_synced(&self) -> bool {
        true
    }
    fn lookup_transaction_by_hash(
        &self,
        hash: Uint256,
        ledger_range: Option<(u32, u32)>,
    ) -> Result<TxLookupOutcome, TxLookupError>;
    fn lookup_transaction_by_ctid(
        &self,
        ledger_seq: u32,
        txn_index: u16,
        ledger_range: Option<(u32, u32)>,
    ) -> Result<TxLookupOutcome, TxLookupError>;
}

enum TxSelector {
    Hash(Uint256),
    Ctid { ledger_seq: u32, txn_index: u16 },
}

fn ensure_object(value: &mut JsonValue) -> &mut BTreeMap<String, JsonValue> {
    if !matches!(value, JsonValue::Object(_)) {
        *value = JsonValue::Object(BTreeMap::new());
    }

    let JsonValue::Object(object) = value else {
        unreachable!("json value should be an object");
    };
    object
}

fn parse_bool_flag(params: &JsonValue, field: &str) -> bool {
    let JsonValue::Object(object) = params else {
        return false;
    };
    matches!(object.get(field), Some(JsonValue::Bool(true)))
}

fn parse_u32_range_value(value: &JsonValue) -> Result<u32, JsonValue> {
    match value {
        JsonValue::Unsigned(value) => {
            u32::try_from(*value).map_err(|_| rpc_error(RpcErrorCode::InvalidLedgerRange))
        }
        JsonValue::Signed(value) if *value >= 0 => {
            u32::try_from(*value as u64).map_err(|_| rpc_error(RpcErrorCode::InvalidLedgerRange))
        }
        _ => Err(rpc_error(RpcErrorCode::InvalidLedgerRange)),
    }
}

fn parse_ledger_range(params: &JsonValue) -> Result<Option<(u32, u32)>, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Ok(None);
    };

    let (Some(min), Some(max)) = (object.get("min_ledger"), object.get("max_ledger")) else {
        return Ok(None);
    };

    let min = parse_u32_range_value(min)?;
    let max = parse_u32_range_value(max)?;
    if max < min {
        return Err(rpc_error(RpcErrorCode::InvalidLedgerRange));
    }
    if max - min > 1_000 {
        return Err(rpc_error(RpcErrorCode::ExcessiveLedgerRange));
    }
    Ok(Some((min, max)))
}

fn parse_selector<S: TxSource>(params: &JsonValue, source: &S) -> Result<TxSelector, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Err(rpc_error(RpcErrorCode::InvalidParams));
    };

    let has_transaction = object.contains_key("transaction");
    let has_ctid = object.contains_key("ctid");
    if has_transaction == has_ctid {
        return Err(rpc_error(RpcErrorCode::InvalidParams));
    }

    if let Some(transaction) = object.get("transaction") {
        let JsonValue::String(transaction) = transaction else {
            return Err(rpc_error(RpcErrorCode::NotImpl));
        };
        let hash = Uint256::from_hex(transaction).map_err(|_| rpc_error(RpcErrorCode::NotImpl))?;
        return Ok(TxSelector::Hash(hash));
    }

    let JsonValue::String(ctid) = object.get("ctid").expect("ctid selector must be present") else {
        return Err(rpc_error(RpcErrorCode::InvalidParams));
    };

    let Some((ledger_seq, txn_index, network_id)) = decode_ctid(ctid.as_str()) else {
        return Err(rpc_error(RpcErrorCode::InvalidParams));
    };

    if u32::from(network_id) != source.network_id() {
        return Err(make_error_message(
            RpcErrorCode::WrongNetwork,
            format!(
                "Wrong network. You should submit this request to a node running on NetworkID: {}",
                network_id
            ),
        ));
    }

    Ok(TxSelector::Ctid {
        ledger_seq,
        txn_index,
    })
}

fn make_transaction_owner(record: &TxRecord) -> Transaction {
    let mut owner = Transaction::new(Arc::clone(&record.txn));
    owner.set_status_with_ledger(TransStatus::COMMITTED, record.ledger_index, None, None);
    owner
}

fn compute_ctid(record: &TxRecord) -> Option<String> {
    let (Some(txn_index), Some(network_id)) = (record.txn_index, record.network_id) else {
        return None;
    };
    encode_ctid(record.ledger_index, txn_index, network_id)
}

fn insert_meta(
    response: &mut JsonValue,
    record: &TxRecord,
    binary: bool,
    api_version: u32,
    txn: &STTx,
) {
    let Some(meta) = record.meta.as_ref() else {
        return;
    };

    if binary {
        let key = if api_version > 1 { "meta_blob" } else { "meta" };
        ensure_object(response).insert(
            key.to_owned(),
            JsonValue::String(str_hex(meta.get_as_object().get_serializer().data())),
        );
        return;
    }

    let mut meta_json = meta.get_json(JsonOptions::NONE);
    insert_delivered_amount(
        &mut meta_json,
        record.ledger_index,
        record.close_time.map(|close_time| close_time.as_seconds()),
        txn,
        meta,
    );
    insert_mp_token_issuance_id(&mut meta_json, txn, meta);
    ensure_object(response).insert("meta".to_owned(), meta_json);
    insert_nft_synthetic_in_json(response, txn, meta);
}

fn insert_v2_response(response: &mut JsonValue, record: &TxRecord, binary: bool) {
    let txn = record.txn.as_ref();
    let owner = make_transaction_owner(record);
    let object = ensure_object(response);

    if binary {
        object.insert(
            "tx_blob".to_owned(),
            owner.get_json_with_close_time(
                JsonOptions::INCLUDE_DATE | JsonOptions::DISABLE_API_PRIOR_V2,
                true,
                record
                    .close_time
                    .map(|close_time| i64::from(close_time.as_seconds())),
            ),
        );
    } else {
        let mut tx_json = owner.get_json_with_close_time(
            JsonOptions::INCLUDE_DATE | JsonOptions::DISABLE_API_PRIOR_V2,
            false,
            record
                .close_time
                .map(|close_time| i64::from(close_time.as_seconds())),
        );
        insert_deliver_max(&mut tx_json, txn.get_txn_type(), 2);
        object.insert("tx_json".to_owned(), tx_json);
    }

    object.insert(
        "hash".to_owned(),
        JsonValue::String(txn.get_transaction_id().to_string()),
    );

    if let Some(ledger_hash) = record.ledger_hash {
        object.insert(
            "ledger_hash".to_owned(),
            JsonValue::String(ledger_hash.to_string()),
        );
    }

    if record.validated {
        object.insert(
            "ledger_index".to_owned(),
            JsonValue::Unsigned(u64::from(record.ledger_index)),
        );
        if let Some(close_time) = record.close_time {
            object.insert(
                "close_time_iso".to_owned(),
                JsonValue::String(to_string_iso(close_time)),
            );
        }
    }

    object.insert("validated".to_owned(), JsonValue::Bool(record.validated));
    if let Some(ctid) = compute_ctid(record) {
        object.insert("ctid".to_owned(), JsonValue::String(ctid));
    }

    insert_meta(response, record, binary, 2, txn);
}

fn insert_v1_response(response: &mut JsonValue, record: &TxRecord, binary: bool) {
    let txn = record.txn.as_ref();
    let owner = make_transaction_owner(record);
    *response = owner.get_json_with_close_time(
        JsonOptions::INCLUDE_DATE,
        binary,
        record
            .close_time
            .map(|close_time| i64::from(close_time.as_seconds())),
    );

    if !binary {
        insert_deliver_max(response, txn.get_txn_type(), 1);
    }

    ensure_object(response).insert("validated".to_owned(), JsonValue::Bool(record.validated));
    if let Some(ctid) = compute_ctid(record) {
        ensure_object(response).insert("ctid".to_owned(), JsonValue::String(ctid));
    }
    insert_meta(response, record, binary, 1, txn);
}

fn render_found(record: &TxRecord, binary: bool, api_version: u32) -> JsonValue {
    let mut response = JsonValue::Object(BTreeMap::new());
    if api_version > 1 {
        insert_v2_response(&mut response, record, binary);
    } else {
        insert_v1_response(&mut response, record, binary);
    }
    response
}

fn render_not_found(searched: TxSearched) -> JsonValue {
    let mut response = JsonValue::Object(BTreeMap::new());
    if searched != TxSearched::Unknown {
        ensure_object(&mut response).insert(
            "searched_all".to_owned(),
            JsonValue::Bool(searched == TxSearched::All),
        );
    }
    crate::inject_error(RpcErrorCode::TxnNotFound, &mut response);
    response
}

pub fn do_tx<S: TxSource>(request: &TxRequest<'_>, source: &S) -> JsonValue {
    tracing::trace!(target: "rpc", method = "tx", "tx query");
    if !source.tx_tables_enabled() {
        return rpc_error(RpcErrorCode::NotEnabled);
    }
    if !source.network_synced() {
        return rpc_error(RpcErrorCode::NoNetwork);
    }

    let selector = match parse_selector(request.params, source) {
        Ok(selector) => selector,
        Err(error) => return error,
    };
    let ledger_range = match parse_ledger_range(request.params) {
        Ok(range) => range,
        Err(error) => return error,
    };
    let binary = parse_bool_flag(request.params, "binary");

    let result = match selector {
        TxSelector::Hash(hash) => source.lookup_transaction_by_hash(hash, ledger_range),
        TxSelector::Ctid {
            ledger_seq,
            txn_index,
        } => source.lookup_transaction_by_ctid(ledger_seq, txn_index, ledger_range),
    };

    match result {
        Ok(TxLookupOutcome::Found(record)) => render_found(&record, binary, request.api_version),
        Ok(TxLookupOutcome::NotFound(searched)) => render_not_found(searched),
        Err(TxLookupError::DatabaseDeserialization) => rpc_error(RpcErrorCode::DbDeserialization),
    }
}
