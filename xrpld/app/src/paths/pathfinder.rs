//! Pathfinding request parser and source seam.

use std::collections::BTreeMap;

use protocol::{JsonValue, parse_base58_account_id};

use xrpld_core::{RpcErrorCode, Status};

#[derive(Debug, Clone, PartialEq)]
pub struct PathFinderRequest {
    pub source_account: String,
    pub destination_account: String,
    pub destination_amount: JsonValue,
    pub send_max: Option<JsonValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathFindTuning {
    pub old: u32,
    pub search: u32,
    pub fast: u32,
    pub max: u32,
}

impl Default for PathFindTuning {
    fn default() -> Self {
        Self {
            old: 2,
            search: 2,
            fast: 2,
            max: 3,
        }
    }
}

pub trait PathFinderSource {
    fn path_find_tuning(&self) -> PathFindTuning {
        PathFindTuning::default()
    }

    fn find_paths(
        &self,
        request: &PathFinderRequest,
        params: &JsonValue,
        search_level: u32,
        is_legacy: bool,
    ) -> Result<JsonValue, Status>;
}

fn amount_like_json_is_valid(value: &JsonValue) -> bool {
    match value {
        JsonValue::String(text) => !text.is_empty(),
        JsonValue::Object(object) => {
            matches!(object.get("currency"), Some(JsonValue::String(currency)) if !currency.is_empty())
                && matches!(object.get("value"), Some(JsonValue::String(amount)) if !amount.is_empty())
                && match object.get("issuer") {
                    None => true,
                    Some(JsonValue::String(account)) => parse_base58_account_id(account).is_some(),
                    Some(_) => false,
                }
        }
        _ => false,
    }
}

pub fn parse_path_finder_request(params: &JsonValue) -> Result<PathFinderRequest, Status> {
    let JsonValue::Object(object) = params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    let source_account = match object.get("source_account") {
        Some(JsonValue::String(value)) if parse_base58_account_id(value).is_some() => value.clone(),
        Some(JsonValue::String(_)) => return Err(Status::new(RpcErrorCode::SrcActNotFound)),
        Some(_) => return Err(Status::invalid_field_error("source_account")),
        None => return Err(Status::new(RpcErrorCode::SrcActMissing)),
    };

    let destination_account = match object.get("destination_account") {
        Some(JsonValue::String(value)) if parse_base58_account_id(value).is_some() => value.clone(),
        Some(JsonValue::String(_)) => return Err(Status::new(RpcErrorCode::DstActNotFound)),
        Some(_) => return Err(Status::invalid_field_error("destination_account")),
        None => return Err(Status::missing_field_error("destination_account")),
    };

    let Some(destination_amount) = object.get("destination_amount") else {
        return Err(Status::missing_field_error("destination_amount"));
    };
    if !amount_like_json_is_valid(destination_amount) {
        return Err(Status::new(RpcErrorCode::DstAmtMalformed));
    }

    let send_max = object.get("send_max").cloned();
    if let Some(send_max_value) = send_max.as_ref() {
        if !amount_like_json_is_valid(send_max_value) {
            return Err(Status::new(RpcErrorCode::DstAmtMalformed));
        }
    }

    Ok(PathFinderRequest {
        source_account,
        destination_account,
        destination_amount: destination_amount.clone(),
        send_max,
    })
}

pub fn make_path_find_status(
    request_id: u64,
    request: &PathFinderRequest,
    result: JsonValue,
    full_reply: bool,
    is_legacy: bool,
) -> JsonValue {
    let mut response = BTreeMap::from([
        ("id".to_owned(), JsonValue::Unsigned(request_id)),
        (
            "source_account".to_owned(),
            JsonValue::String(request.source_account.clone()),
        ),
        (
            "destination_account".to_owned(),
            JsonValue::String(request.destination_account.clone()),
        ),
        (
            "destination_amount".to_owned(),
            request.destination_amount.clone(),
        ),
        ("full_reply".to_owned(), JsonValue::Bool(full_reply)),
        ("alternatives".to_owned(), result),
    ]);
    if is_legacy {
        response.insert(
            "destination_currencies".to_owned(),
            JsonValue::Array(Vec::new()),
        );
    }
    JsonValue::Object(response)
}
