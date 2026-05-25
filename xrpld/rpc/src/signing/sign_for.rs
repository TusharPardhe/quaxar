//! `sign_for` handler port from `xrpld/rpc/handlers/admin/signing/the reference source`.

use crate::commands::rpc_helpers::transaction_sign_for;
use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::Status;
use protocol::JsonValue;

pub struct SignForSource;

pub fn do_sign_for<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, SignForSource, Runtime>,
) -> Result<JsonValue, Status> {
    transaction_sign_for(ctx)
}
