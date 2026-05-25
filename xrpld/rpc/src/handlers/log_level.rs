//! `log_level` handler port from `xrpld/rpc/handlers/admin/log/the reference source`.

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::Status;
use protocol::JsonValue;

pub struct LogLevelSource;

pub fn do_log_level<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, LogLevelSource, Runtime>,
) -> Result<JsonValue, Status> {
    let JsonValue::Object(params) = ctx.params else {
        return Ok(ctx.runtime.log_level_get());
    };

    if let Some(severity) = params.get("severity") {
        let severity = severity
            .as_str()
            .ok_or_else(|| Status::expected_field_error("severity", "string"))?;
        let partition = params
            .get("partition")
            .and_then(|v| v.as_str())
            .unwrap_or("base");
        let status = ctx
            .runtime
            .log_level_set(partition.to_string(), severity.to_string());
        if !status.is_ok() {
            return Err(status);
        }
    }

    Ok(ctx.runtime.log_level_get())
}
