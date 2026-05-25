//! `connect` handler port from `xrpld/rpc/handlers/admin/peer/the reference source`.

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::{RpcErrorCode, Status};
use protocol::JsonValue;

pub struct ConnectSource;

pub fn do_connect<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, ConnectSource, Runtime>,
) -> Result<JsonValue, Status> {
    tracing::info!(target: "rpc", method = "connect", "Peer connect requested via RPC");
    let JsonValue::Object(params) = ctx.params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    let Some(JsonValue::String(ip)) = params.get("ip") else {
        return Err(Status::expected_field_error("ip", "string"));
    };

    let port = params
        .get("port")
        .and_then(|v| v.as_u64())
        .map(|v| v as u16)
        .ok_or_else(|| Status::expected_field_error("port", "number"))?;

    let status = ctx.runtime.peer_connect(ip.clone(), port);
    if !status.is_ok() {
        return Err(status);
    }

    Ok(protocol::json!({
        "message": "connecting"
    }))
}
