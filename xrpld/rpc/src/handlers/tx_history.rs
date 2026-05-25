//! Transaction history RPC handler slice.

use std::collections::BTreeMap;

use app::Transaction;
use protocol::{JsonOptions, JsonValue, STTx, StBase, TxType};

use crate::commands::rpc_helpers::rpc_error;
use crate::handlers::deliver_max::insert_deliver_max;
use crate::handlers::ledger_lookup::{RpcErrorCode, RpcRole, is_unlimited};
use crate::state::tx_support::TxHistoryRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxHistoryRequest<'a> {
    pub params: &'a JsonValue,
    pub role: RpcRole,
    pub api_version: u32,
}

pub trait TxHistoryTransaction {
    fn get_json(&self) -> JsonValue;
    fn get_txn_type(&self) -> TxType;
}

impl TxHistoryTransaction for STTx {
    fn get_json(&self) -> JsonValue {
        self.json(JsonOptions::NONE)
    }

    fn get_txn_type(&self) -> TxType {
        STTx::get_txn_type(self)
    }
}

pub trait TxHistorySource {
    type Row: TxHistoryTransaction;

    fn tx_tables_enabled(&self) -> bool;
    fn get_tx_history(&self, start_index: u32) -> Vec<Self::Row>;
}

impl TxHistoryTransaction for TxHistoryRow {
    fn get_json(&self) -> JsonValue {
        self.transaction.get_json(JsonOptions::NONE, false)
    }

    fn get_txn_type(&self) -> TxType {
        self.transaction.get_s_transaction().get_txn_type()
    }
}

impl TxHistoryTransaction for Transaction {
    fn get_json(&self) -> JsonValue {
        self.get_json(JsonOptions::NONE, false)
    }

    fn get_txn_type(&self) -> TxType {
        self.get_s_transaction().get_txn_type()
    }
}

fn parse_start_index(params: &JsonValue) -> Result<u32, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Err(rpc_error(RpcErrorCode::InvalidParams));
    };

    let Some(start) = object.get("start") else {
        return Err(rpc_error(RpcErrorCode::InvalidParams));
    };

    match start {
        JsonValue::Unsigned(value) => {
            u32::try_from(*value).map_err(|_| rpc_error(RpcErrorCode::InvalidParams))
        }
        JsonValue::Signed(value) if *value >= 0 => {
            u32::try_from(*value as u64).map_err(|_| rpc_error(RpcErrorCode::InvalidParams))
        }
        _ => Err(rpc_error(RpcErrorCode::InvalidParams)),
    }
}

pub fn do_tx_history<S: TxHistorySource>(request: &TxHistoryRequest<'_>, source: &S) -> JsonValue {
    if !source.tx_tables_enabled() {
        return rpc_error(RpcErrorCode::NotEnabled);
    }

    let start_index = match parse_start_index(request.params) {
        Ok(start_index) => start_index,
        Err(error) => return error,
    };

    if start_index > 10_000 && !is_unlimited(request.role) {
        return rpc_error(RpcErrorCode::NoPermission);
    }

    let txs = source.get_tx_history(start_index);
    let rendered = txs
        .into_iter()
        .map(|tx| {
            let txn_type = tx.get_txn_type();
            let mut tx_json = tx.get_json();
            insert_deliver_max(&mut tx_json, txn_type, request.api_version);
            tx_json
        })
        .collect::<Vec<_>>();

    JsonValue::Object(BTreeMap::from([
        (
            "index".to_owned(),
            JsonValue::Unsigned(u64::from(start_index)),
        ),
        ("txs".to_owned(), JsonValue::Array(rendered)),
    ]))
}
