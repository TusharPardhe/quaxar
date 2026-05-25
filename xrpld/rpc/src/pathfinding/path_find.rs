//! `path_find` and legacy `ripple_path_find` handlers.

use std::collections::BTreeMap;

use app::paths::{PathFindSession, PathFinderSource, PathRequestManager};
use protocol::JsonValue;

use crate::state::context::RpcRuntime;
use crate::status::{RpcErrorCode, Status};

fn status_json(status: Status) -> JsonValue {
    let mut value = JsonValue::Object(BTreeMap::new());
    status.inject(&mut value);
    value
}

pub fn do_path_find<
    S: PathFinderSource + ?Sized,
    Runtime: RpcRuntime + ?Sized,
    Session: PathFindSession + ?Sized,
>(
    params: &JsonValue,
    api_version: u32,
    runtime: &Runtime,
    session: Option<&Session>,
    manager: &PathRequestManager,
    source: &S,
    closed_ledger_index: u32,
) -> JsonValue {
    tracing::debug!(target: "rpc", method = "path_find", "path_find query");
    if runtime.path_search_max() == 0 {
        return status_json(Status::new(RpcErrorCode::NotSupported));
    }
    if !runtime.network_synced() {
        return status_json(Status::new(RpcErrorCode::NoNetwork));
    }

    let Some(session) = session else {
        return status_json(Status::new(RpcErrorCode::NoEvents));
    };

    let JsonValue::Object(object) = params else {
        return status_json(Status::new(RpcErrorCode::InvalidParams));
    };

    let Some(JsonValue::String(subcommand)) = object.get("subcommand") else {
        return status_json(Status::new(RpcErrorCode::InvalidParams));
    };

    session.set_api_version(api_version);

    let result = match subcommand.as_str() {
        "create" => manager.make_path_request(session, source, closed_ledger_index, params),
        "close" => manager.close_request(session),
        "status" => manager.status_request(session),
        _ => Err(Status::new(RpcErrorCode::InvalidParams)),
    };

    result.unwrap_or_else(status_json)
}

pub fn do_ripple_path_find<
    S: PathFinderSource + ?Sized,
    Runtime: RpcRuntime + ?Sized,
    Session: PathFindSession + ?Sized,
>(
    params: &JsonValue,
    runtime: &Runtime,
    session: Option<&Session>,
    manager: &PathRequestManager,
    source: &S,
    ledger_index: u32,
    has_explicit_ledger: bool,
) -> JsonValue {
    if runtime.path_search_max() == 0 {
        return status_json(Status::new(RpcErrorCode::NotSupported));
    }

    let result = if has_explicit_ledger || !runtime.network_synced() {
        manager.direct_legacy_path_request(source, ledger_index, params)
    } else if let Some(session) = session {
        manager.make_legacy_path_request(session, source, ledger_index, params)
    } else {
        manager.direct_legacy_path_request(source, ledger_index, params)
    };

    result.unwrap_or_else(status_json)
}
