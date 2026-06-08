//! `export_snapshot` admin RPC handler — triggers a live snapshot export
//! using the running node's backend.

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::{RpcErrorCode, Status};
use protocol::JsonValue;

pub struct ExportSnapshotSource;

pub fn do_export_snapshot<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, ExportSnapshotSource, Runtime>,
) -> Result<JsonValue, Status> {
    let JsonValue::Object(params) = ctx.params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    let output = params
        .get("output")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Status::expected_field_error("output", "string"))?;

    ctx.runtime
        .export_snapshot(output)
        .map_err(|e| Status::with_message(RpcErrorCode::Internal, &e))
}
