//! Ledger header RPC handler slice.

use basics::str_hex::str_hex;
use protocol::{
    JsonValue, LedgerHeader, serialize_ledger_header as serialize_protocol_ledger_header,
};
use std::collections::BTreeMap;

use crate::handlers::ledger_lookup::RpcStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerHeaderResolved {
    pub base_json: JsonValue,
    pub header: LedgerHeader,
}

pub trait LedgerHeaderSource {
    fn resolve_ledger_header(&self) -> Result<LedgerHeaderResolved, RpcStatus>;
}

pub fn serialize_ledger_header(header: &LedgerHeader) -> Vec<u8> {
    serialize_protocol_ledger_header(header, false)
}

fn ensure_object(json: &mut JsonValue) -> &mut BTreeMap<String, JsonValue> {
    if !matches!(json, JsonValue::Object(_)) {
        *json = JsonValue::Object(BTreeMap::new());
    }

    let JsonValue::Object(object) = json else {
        unreachable!("json value should now be an object");
    };
    object
}

pub fn do_ledger_header<S: LedgerHeaderSource>(source: &S) -> JsonValue {
    let resolved = match source.resolve_ledger_header() {
        Ok(resolved) => resolved,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
    };

    let mut result = resolved.base_json;
    let object = ensure_object(&mut result);
    object.insert(
        "ledger_data".to_owned(),
        JsonValue::String(str_hex(serialize_ledger_header(&resolved.header))),
    );
    result
}
