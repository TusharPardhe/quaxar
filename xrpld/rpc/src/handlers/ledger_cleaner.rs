//! `ledger_cleaner` handler port from `xrpld/rpc/handlers/admin/data/the reference source`.

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::Status;
use protocol::JsonValue;

pub struct LedgerCleanerSource;

pub fn do_ledger_cleaner<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, LedgerCleanerSource, Runtime>,
) -> Result<JsonValue, Status> {
    let status = ctx.runtime.ledger_cleaner_trigger(ctx.params);
    if !status.is_ok() {
        return Err(status);
    }
    Ok(protocol::json!({
        "message": "ledger cleaner triggered"
    }))
}
