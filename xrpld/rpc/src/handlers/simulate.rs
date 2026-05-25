//! `simulate` handler port from `xrpld/rpc/handlers/transaction/the reference source`.

use crate::commands::rpc_helpers::{
    autofill_tx, get_tx_json_from_params, parse_sttx_from_params, simulate_txn,
};
use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::Status;
use protocol::JsonValue;

pub struct SimulateSource;

pub fn do_simulate<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, SimulateSource, Runtime>,
) -> Result<JsonValue, Status> {
    let st_tx = match ctx.params {
        JsonValue::Object(object) if object.contains_key(protocol::jss::tx_json) => {
            let mut tx_json = get_tx_json_from_params(ctx.params)?;
            autofill_tx(&mut tx_json, ctx)?;
            parse_sttx_from_params(&JsonValue::Object(
                [(protocol::jss::tx_json.to_owned(), tx_json)]
                    .into_iter()
                    .collect(),
            ))?
        }
        _ => parse_sttx_from_params(ctx.params)?,
    };

    let result = simulate_txn(ctx, &st_tx)?;
    if let JsonValue::Object(ref obj) = result {
        if let Some(JsonValue::String(engine_result)) = obj.get(protocol::jss::engine_result) {
            tracing::debug!(target: "rpc", result = %engine_result, "Transaction simulated");
        }
    }
    Ok(result)
}
