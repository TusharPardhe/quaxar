//! `stop` handler port from `xrpld/rpc/handlers/admin/server_control/the reference source`.

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::Status;
use protocol::JsonValue;

pub struct StopSource;

pub fn do_stop<Runtime: RpcRuntime>(
    _ctx: &RpcRequestContext<'_, StopSource, Runtime>,
) -> Result<JsonValue, Status> {
    tracing::info!(target: "rpc", method = "stop", "Server stop requested via RPC");
    let status = _ctx.runtime.stop();
    if !status.is_ok() {
        return Err(status);
    }
    Ok(protocol::json!({
        "message": "server stopping"
    }))
}
