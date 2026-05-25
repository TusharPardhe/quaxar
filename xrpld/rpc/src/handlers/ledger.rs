//! Narrow `ledger` RPC handler port.
//!
//! This keeps the reference owner-level control flow from the reference implementation while leaving
//! concrete ledger rendering, fee-track state, and queue payload production in
//! explicit source seams instead of a fake `Application` graph.

use std::collections::BTreeMap;

use ledger::LedgerFillOptions;
use protocol::JsonValue;

use crate::handlers::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcErrorCode, RpcRole, RpcStatus,
    is_unlimited, lookup_ledger_with_result,
};

pub const WARN_RPC_FIELDS_DEPRECATED: i64 = 2004;
const TYPE_DEPRECATED_MESSAGE: &str = "Some fields from your request are deprecated. Please check the documentation at https://xrpl.org/docs/references/http-websocket-apis/ and update your request. Field `type` is deprecated.";

pub trait LedgerSource: LedgerLookupSource {
    fn fee_track_loaded_local(&self) -> bool {
        false
    }

    fn render_selected_ledger(
        &self,
        ledger: LedgerLookupLedger,
        options: LedgerFillOptions,
    ) -> Result<JsonValue, RpcStatus>;

    fn render_closed_ledger(&self) -> Result<JsonValue, RpcStatus>;

    fn render_open_ledger(&self) -> Result<JsonValue, RpcStatus>;
}

fn ensure_object(json: &mut JsonValue) -> &mut BTreeMap<String, JsonValue> {
    if matches!(json, JsonValue::Null) {
        *json = JsonValue::Object(BTreeMap::new());
    }

    let JsonValue::Object(object) = json else {
        panic!("ledger rpc result must be an object or null");
    };
    object
}

fn merge_objects(target: &mut JsonValue, source: &JsonValue) {
    let JsonValue::Object(source) = source else {
        panic!("ledger rpc merge source must be an object");
    };

    let target = ensure_object(target);
    for (key, value) in source {
        target.insert(key.clone(), value.clone());
    }
}

fn normalize_ledger_index_string(value: &mut JsonValue) {
    let JsonValue::Object(object) = value else {
        return;
    };
    let Some(index) = object.get_mut("ledger_index") else {
        return;
    };
    match index {
        JsonValue::Unsigned(number) => *index = JsonValue::String(number.to_string()),
        JsonValue::Signed(number) => *index = JsonValue::String(number.to_string()),
        _ => {}
    }
}

fn parse_flag(params: &JsonValue, field: &str) -> bool {
    let JsonValue::Object(object) = params else {
        return false;
    };

    matches!(object.get(field), Some(JsonValue::Bool(true)))
}

fn has_field(params: &JsonValue, field: &str) -> bool {
    let JsonValue::Object(object) = params else {
        return false;
    };

    object.contains_key(field)
}

fn needs_ledger(params: &JsonValue) -> bool {
    has_field(params, "ledger")
        || has_field(params, "ledger_hash")
        || has_field(params, "ledger_index")
}

fn build_options(params: &JsonValue) -> LedgerFillOptions {
    let full = parse_flag(params, "full");
    let transactions = parse_flag(params, "transactions");
    let accounts = parse_flag(params, "accounts");
    let expand = parse_flag(params, "expand");
    let binary = parse_flag(params, "binary");
    let owner_funds = parse_flag(params, "owner_funds");
    let queue = parse_flag(params, "queue");

    let mut options = LedgerFillOptions::new(0);
    if full {
        options |= LedgerFillOptions::FULL;
    }
    if expand {
        options |= LedgerFillOptions::EXPAND;
    }
    if transactions {
        options |= LedgerFillOptions::DUMP_TXRP;
    }
    if accounts {
        options |= LedgerFillOptions::DUMP_STATE;
    }
    if binary {
        options |= LedgerFillOptions::BINARY;
    }
    if owner_funds {
        options |= LedgerFillOptions::OWNER_FUNDS;
    }
    if queue {
        options |= LedgerFillOptions::DUMP_QUEUE;
    }
    options
}

fn inject_type_warning(params: &JsonValue, result: &mut JsonValue) {
    if !has_field(params, "type") {
        return;
    }

    let warnings = JsonValue::Array(vec![JsonValue::Object(BTreeMap::from([
        (
            "id".to_owned(),
            JsonValue::Signed(WARN_RPC_FIELDS_DEPRECATED),
        ),
        (
            "message".to_owned(),
            JsonValue::String(TYPE_DEPRECATED_MESSAGE.to_owned()),
        ),
    ]))]);

    ensure_object(result).insert("warnings".to_owned(), warnings);
}

fn render_selected<S: LedgerSource>(
    params: &JsonValue,
    role: RpcRole,
    api_version: u32,
    source: &S,
) -> Result<JsonValue, RpcStatus> {
    let context = LedgerLookupContext {
        params,
        source,
        api_version,
        role,
    };
    let (ledger, mut result) = lookup_ledger_with_result(&context)?;
    let options = build_options(params);

    if options.contains(LedgerFillOptions::FULL) || options.contains(LedgerFillOptions::DUMP_STATE)
    {
        if !is_unlimited(role) {
            return Err(RpcStatus::new(RpcErrorCode::NoPermission));
        }

        if source.fee_track_loaded_local() && !is_unlimited(role) {
            return Err(RpcStatus::new(RpcErrorCode::TooBusy));
        }
    }

    if options.contains(LedgerFillOptions::DUMP_QUEUE) && !ledger.open {
        return Err(RpcStatus::new(RpcErrorCode::InvalidParams));
    }

    let mut rendered = source.render_selected_ledger(ledger, options)?;
    normalize_ledger_index_string(&mut rendered);
    if matches!(&rendered, JsonValue::Object(object) if object.contains_key("ledger")) {
        merge_objects(&mut result, &rendered);
    } else {
        ensure_object(&mut result).insert("ledger".to_owned(), rendered);
    }
    inject_type_warning(params, &mut result);
    Ok(result)
}

fn render_default<S: LedgerSource>(params: &JsonValue, source: &S) -> Result<JsonValue, RpcStatus> {
    let mut result = JsonValue::Object(BTreeMap::new());
    let closed = source.render_closed_ledger()?;
    let open = source.render_open_ledger()?;
    ensure_object(&mut result).insert("closed".to_owned(), closed);
    ensure_object(&mut result).insert("open".to_owned(), open);
    inject_type_warning(params, &mut result);
    Ok(result)
}

pub fn do_ledger<S: LedgerSource>(
    params: &JsonValue,
    role: RpcRole,
    api_version: u32,
    source: &S,
) -> JsonValue {
    if let JsonValue::Object(obj) = params {
        let ledger_index = obj
            .get("ledger_index")
            .and_then(|v| match v {
                JsonValue::Unsigned(n) => Some(n.to_string()),
                JsonValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default();
        tracing::trace!(target: "rpc", ledger_index = %ledger_index, "ledger query");
    }

    let response = if needs_ledger(params) {
        render_selected(params, role, api_version, source)
    } else {
        render_default(params, source)
    };

    match response {
        Ok(result) => result,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            error
        }
    }
}
