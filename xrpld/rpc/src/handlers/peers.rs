//! `peers` handler port from `xrpld/rpc/handlers/admin/peer/the reference source`.

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::Status;
use protocol::JsonValue;

pub struct PeersSource;

pub fn do_peers<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, PeersSource, Runtime>,
) -> Result<JsonValue, Status> {
    Ok(ctx.runtime.peers_get())
}
