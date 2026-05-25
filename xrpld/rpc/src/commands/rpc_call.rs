//! RPC client-side request shapers aligned with the reference implementation.

use std::collections::BTreeMap;

use protocol::{JsonValue, parse_base58_account_id};

use crate::status::{RpcErrorCode, Status};

pub fn create_http_post(
    host: &str,
    path: &str,
    body: &str,
    headers: &BTreeMap<String, String>,
) -> String {
    let mut request = String::new();
    let request_path = if path.is_empty() { "/" } else { path };
    request.push_str(&format!("POST {request_path} HTTP/1.0\r\n"));
    request.push_str("User-Agent: xrpld-rust-json-rpc/v1\r\n");
    request.push_str(&format!("Host: {host}\r\n"));
    request.push_str("Content-Type: application/json\r\n");
    request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    request.push_str("Accept: application/json\r\n");
    for (key, value) in headers {
        request.push_str(key);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.push_str(body);
    request
}

fn parse_ledger_selector(value: &str) -> JsonValue {
    match value {
        "current" | "closed" | "validated" | "" => JsonValue::String(value.to_owned()),
        other if other.len() == 64 => JsonValue::String(other.to_owned()),
        other => other
            .parse::<u64>()
            .map(JsonValue::Unsigned)
            .unwrap_or_else(|_| JsonValue::String(other.to_owned())),
    }
}

pub fn rpc_cmd_to_json(args: &[String], _api_version: u32) -> Result<JsonValue, Status> {
    let Some(method) = args.first() else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    let mut request = BTreeMap::new();
    request.insert("command".to_owned(), JsonValue::String(method.clone()));

    match method.as_str() {
        "ledger" => {
            if let Some(selector) = args.get(1) {
                let parsed = parse_ledger_selector(selector);
                match &parsed {
                    JsonValue::String(text) if text.len() == 64 => {
                        request.insert("ledger_hash".to_owned(), parsed);
                    }
                    _ => {
                        request.insert("ledger_index".to_owned(), parsed);
                    }
                }
            }
        }
        "server_info" | "server_state" => {
            if let Some(flag) = args.get(1)
                && flag == "counters"
            {
                request.insert("counters".to_owned(), JsonValue::Bool(true));
            }
        }
        "account_tx" => {
            let Some(account) = args.get(1) else {
                return Err(Status::missing_field_error("account"));
            };
            if parse_base58_account_id(account).is_none() {
                return Err(Status::new(RpcErrorCode::ActMalformed));
            }
            request.insert("account".to_owned(), JsonValue::String(account.clone()));
            if let Some(value) = args.get(2)
                && let Ok(ledger_min) = value.parse::<u64>()
            {
                request.insert(
                    "ledger_index_min".to_owned(),
                    JsonValue::Unsigned(ledger_min),
                );
            }
            if let Some(value) = args.get(3)
                && let Ok(ledger_max) = value.parse::<u64>()
            {
                request.insert(
                    "ledger_index_max".to_owned(),
                    JsonValue::Unsigned(ledger_max),
                );
            }
            if let Some(value) = args.get(4)
                && let Ok(limit) = value.parse::<u64>()
            {
                request.insert("limit".to_owned(), JsonValue::Unsigned(limit));
            }
            for flag in &args[5..] {
                match flag.as_str() {
                    "binary" => {
                        request.insert("binary".to_owned(), JsonValue::Bool(true));
                    }
                    "forward" => {
                        request.insert("forward".to_owned(), JsonValue::Bool(true));
                    }
                    "descending" => {
                        request.insert("forward".to_owned(), JsonValue::Bool(false));
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    Ok(JsonValue::Object(request))
}
