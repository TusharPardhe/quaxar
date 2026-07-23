//! `snapshot_status` admin RPC handler — returns the latest snapshot export job state.

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::Status;
use protocol::JsonValue;

pub struct SnapshotStatusSource;

pub fn do_snapshot_status<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, SnapshotStatusSource, Runtime>,
) -> Result<JsonValue, Status> {
    Ok(ctx.runtime.snapshot_status())
}
