//! Ledger header RPC handler slice (rippled `doLedgerHeader`).

use basics::str_hex::str_hex;
use ledger::LedgerFillOptions;
use protocol::{
    JsonValue, LedgerHeader, serialize_ledger_header as serialize_protocol_ledger_header,
};
use std::collections::BTreeMap;

use crate::handlers::ledger::{LedgerSource, set_ledger_index_type};
use crate::handlers::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, RpcErrorCode, RpcRole, RpcStatus,
    lookup_ledger_with_result,
};

pub struct LedgerHeaderRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait LedgerHeaderSource: LedgerSource {
    /// The header of the ledger the lookup selected.
    fn ledger_header(&self, ledger: &LedgerLookupLedger) -> Option<LedgerHeader>;
}

pub fn serialize_ledger_header(header: &LedgerHeader) -> Vec<u8> {
    serialize_protocol_ledger_header(header, false)
}

fn error_json(status: RpcStatus) -> JsonValue {
    let mut error = JsonValue::Object(BTreeMap::new());
    status.inject(&mut error);
    error
}

/// rippled `doLedgerHeader`: `lookupLedger` (so the request's ledger
/// selector is honored and `ledger_hash`, `ledger_index`, `validated` are
/// set), then `ledger_data` as the hex of the serialized header, then
/// `ledger` from the same LedgerToJson writer as `ledger` with no options.
pub fn do_ledger_header<S: LedgerHeaderSource>(
    request: &LedgerHeaderRequest<'_>,
    source: &S,
) -> JsonValue {
    let context = LedgerLookupContext {
        params: request.params,
        source,
        api_version: request.api_version,
        role: request.role,
    };
    let (ledger, mut result) = match lookup_ledger_with_result(&context) {
        Ok(value) => value,
        Err(status) => return error_json(status),
    };
    let Some(header) = source.ledger_header(&ledger) else {
        return error_json(RpcStatus::new(RpcErrorCode::LedgerNotFound));
    };
    let mut ledger_json =
        match source.render_selected_ledger(ledger, LedgerFillOptions::new(0), request.api_version)
        {
            Ok(json) => json,
            Err(status) => return error_json(status),
        };
    set_ledger_index_type(&mut ledger_json, request.api_version);

    if let JsonValue::Object(object) = &mut result {
        object.insert(
            "ledger_data".to_owned(),
            JsonValue::String(str_hex(serialize_ledger_header(&header))),
        );
        object.insert("ledger".to_owned(), ledger_json);
    }
    result
}
