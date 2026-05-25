//! `peer_reservations_add` handler port from `xrpld/rpc/handlers/admin/peer/the reference source`.

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::{RpcErrorCode, Status};
use protocol::JsonValue;

pub struct PeerReservationsAddSource;

pub fn do_peer_reservations_add<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, PeerReservationsAddSource, Runtime>,
) -> Result<JsonValue, Status> {
    let JsonValue::Object(params) = ctx.params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    let Some(JsonValue::String(pk_str)) = params.get("public_key") else {
        return Err(Status::expected_field_error("public_key", "string"));
    };

    let description = params
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let node_pub = protocol::parse_base58_node_public(pk_str)
        .ok_or_else(|| Status::invalid_field_error("public_key"))?;
    let public_key = protocol::PublicKey::from_slice(&node_pub)
        .map_err(|_| Status::invalid_field_error("public_key"))?;

    let status = ctx
        .runtime
        .peer_reservations_add(public_key, description.to_string());
    if !status.is_ok() {
        return Err(status);
    }

    Ok(protocol::json!({
        "message": "reservation added"
    }))
}
