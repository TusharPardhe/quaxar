//! `ledger_accept` handler port from `xrpld/rpc/handlers/admin/server_control/the reference source`.

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::Status;
use protocol::JsonValue;

pub struct LedgerAcceptSource;

pub fn do_ledger_accept<Runtime: RpcRuntime>(
    _ctx: &RpcRequestContext<'_, LedgerAcceptSource, Runtime>,
) -> Result<JsonValue, Status> {
    tracing::debug!(target: "rpc", method = "ledger_accept", "ledger_accept requested");
    let status = _ctx.runtime.ledger_accept();
    if !status.is_ok() {
        return Err(status);
    }
    Ok(protocol::json!({
        "ledger_current_index": _ctx.runtime.current_ledger_index().unwrap_or_default(),
    }))
}
