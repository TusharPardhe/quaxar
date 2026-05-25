//! `sign` handler port from `xrpld/rpc/handlers/admin/signing/the reference source`.

use crate::commands::rpc_helpers::transaction_sign;
use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::Status;
use protocol::JsonValue;

pub struct SignSource;

pub fn do_sign<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, SignSource, Runtime>,
) -> Result<JsonValue, Status> {
    transaction_sign(ctx)
}
