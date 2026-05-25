//! Narrow `fee` RPC handler port.

use protocol::JsonValue;

use crate::commands::rpc_helpers::rpc_error;
use crate::status::RpcErrorCode;

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
