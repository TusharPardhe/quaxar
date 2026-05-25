//! `channel_authorize` handler port from `xrpld/rpc/handlers/admin/signing/the reference source`.

use crate::commands::rpc_helpers::channel_authorize;
use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::Status;
use protocol::JsonValue;

pub struct ChannelAuthorizeSource;

pub fn do_channel_authorize<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, ChannelAuthorizeSource, Runtime>,
) -> Result<JsonValue, Status> {
    channel_authorize(ctx)
}
