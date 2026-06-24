//! Narrow `fee` RPC handler port.

use std::sync::{Arc, RwLock};

use protocol::JsonValue;

use crate::commands::rpc_helpers::rpc_error;
use crate::status::RpcErrorCode;

pub static FEE_CACHE: RwLock<Option<Arc<[u8]>>> = RwLock::new(None);

pub enum FeeResponse {
    Json(JsonValue),
    PreRendered(Arc<[u8]>),
}

pub fn update_validated_snapshot_cache_fee<S: FeeSource>(source: &S) {
    if !source.network_synced() {
        return;
    }
    let json = match source.fee_json() {
        result @ JsonValue::Object(_) => result,
        _ => return,
    };
    if let Ok(bytes) = serde_json::to_vec(&json) {
        *FEE_CACHE.write().unwrap() = Some(Arc::from(bytes));
    }
}

pub trait FeeSource {
    fn fee_json(&self) -> JsonValue;

    fn network_synced(&self) -> bool {
        true
    }
}

pub fn do_fee<S: FeeSource>(source: &S) -> JsonValue {
    tracing::trace!(target: "rpc", method = "fee", "fee query");
    if !source.network_synced() {
        return rpc_error(RpcErrorCode::NoNetwork);
    }

    match source.fee_json() {
        result @ JsonValue::Object(_) => result,
        _ => rpc_error(RpcErrorCode::Internal),
    }
}

pub fn do_fee_prerendered<S: FeeSource>(source: &S) -> FeeResponse {
    tracing::trace!(target: "rpc", method = "fee", "fee query");
    if !source.network_synced() {
        return FeeResponse::Json(rpc_error(RpcErrorCode::NoNetwork));
    }

    if let Some(cached) = FEE_CACHE.read().unwrap().clone() {
        return FeeResponse::PreRendered(cached);
    }

    match source.fee_json() {
        result @ JsonValue::Object(_) => FeeResponse::Json(result),
        _ => FeeResponse::Json(rpc_error(RpcErrorCode::Internal)),
    }
}
