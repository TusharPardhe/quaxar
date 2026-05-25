//! `log_rotate` handler port from `xrpld/rpc/handlers/admin/log/the reference source`.

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::Status;
use protocol::JsonValue;

pub struct LogRotateSource;

pub fn do_log_rotate<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, LogRotateSource, Runtime>,
) -> Result<JsonValue, Status> {
    let status = ctx.runtime.log_rotate();
    if !status.is_ok() {
        return Err(status);
    }
    Ok(protocol::json!({
        "message": "log rotation triggered"
    }))
}
