//! `peer_reservations_list` handler port from `xrpld/rpc/handlers/admin/peer/the reference source`.

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::Status;
use protocol::JsonValue;

pub struct PeerReservationsListSource;

pub fn do_peer_reservations_list<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, PeerReservationsListSource, Runtime>,
) -> Result<JsonValue, Status> {
    Ok(ctx.runtime.peer_reservations_list())
}
