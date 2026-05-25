//! `can_delete` handler port from `xrpld/rpc/handlers/admin/data/the reference source`.

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::{RpcErrorCode, Status};
use protocol::JsonValue;

pub struct CanDeleteSource;

pub fn do_can_delete<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, CanDeleteSource, Runtime>,
) -> Result<JsonValue, Status> {
    let JsonValue::Object(params) = ctx.params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    if let Some(can_delete) = params.get("can_delete") {
        if let Some(seq_str) = can_delete.as_str() {
            if seq_str == "never" {
                let status = ctx.runtime.can_delete_set(0);
                if !status.is_ok() {
                    return Err(status);
                }
            } else if seq_str == "always" {
                let status = ctx.runtime.can_delete_set(u32::MAX);
                if !status.is_ok() {
                    return Err(status);
                }
            } else if seq_str == "now" {
                // Simplified: in the reference this uses last validated ledger seq
                let status = ctx.runtime.can_delete_set(u32::MAX - 1);
                if !status.is_ok() {
                    return Err(status);
                }
            } else {
                let seq = seq_str
                    .parse::<u32>()
                    .map_err(|_| Status::expected_field_error("can_delete", "number or string"))?;
                let status = ctx.runtime.can_delete_set(seq);
                if !status.is_ok() {
                    return Err(status);
                }
            }
        } else if let Some(seq) = can_delete.as_u64() {
            let status = ctx.runtime.can_delete_set(seq as u32);
            if !status.is_ok() {
                return Err(status);
            }
        } else {
            return Err(Status::expected_field_error(
                "can_delete",
                "number or string",
            ));
        }
    }

    Ok(protocol::json!({
        "can_delete": ctx.runtime.can_delete_get()
    }))
}
