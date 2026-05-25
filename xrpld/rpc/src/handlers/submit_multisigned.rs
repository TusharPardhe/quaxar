//! `submit_multisigned` handler port from `xrpld/rpc/handlers/transaction/the reference source`.

use crate::commands::rpc_helpers::parse_sttx_from_params;
use crate::handlers::submit::{parse_fail_hard, submit_sttx};
use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::Status;
use protocol::JsonValue;

pub struct SubmitMultiSignedSource;

pub fn do_submit_multisigned<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, SubmitMultiSignedSource, Runtime>,
) -> Result<JsonValue, Status> {
    tracing::debug!(target: "rpc", method = "submit_multisigned", "RPC request received");
    let st_tx = parse_sttx_from_params(ctx.params)?;
    let tx_blob_hex = hex::encode(st_tx.get_serializer().data());

    Ok(submit_sttx(
        ctx,
        st_tx,
        &tx_blob_hex,
        parse_fail_hard(ctx.params),
    ))
}
