//! `simulate` handler port from `xrpld/rpc/handlers/transaction/the reference source`.

use crate::commands::rpc_helpers::{
    autofill_tx, get_tx_json_from_params, parse_sttx_from_params, simulate_txn,
};
use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::Status;
use protocol::{JsonValue, TxType, get_field_by_symbol};

pub struct SimulateSource;

pub fn do_simulate<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, SimulateSource, Runtime>,
) -> Result<JsonValue, Status> {
    // `../rippled/src/xrpld/rpc/handlers/transaction/Simulate.cpp::doSimulate`
    // returns RpcNotImpl for ttBATCH before TxQ::apply. Decode either request
    // form first: this also preserves that result for a raw Batch whose Rust
    // STTx parser has not yet reconstructed its cached TxType discriminator.
    if let Ok(JsonValue::Object(tx_json)) = get_tx_json_from_params(ctx.params)
        && matches!(
            tx_json.get("TransactionType"),
            Some(JsonValue::String(txn_type)) if txn_type == "Batch"
        )
    {
        return Err(Status::new(crate::status::RpcErrorCode::NotSupported));
    }

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

    // Raw blobs are parsed before this point. Check the serialized field as
    // well as the cached discriminator: `STTx::from_serial_iter` may not have
    // restored the latter for every valid Batch encoding.
    if st_tx.get_field_u16(get_field_by_symbol("sfTransactionType")) == TxType::BATCH.to_u16() {
        return Err(Status::new(crate::status::RpcErrorCode::NotSupported));
    }

    let result = simulate_txn(ctx, &st_tx)?;
    if let JsonValue::Object(ref obj) = result {
        if let Some(JsonValue::String(engine_result)) = obj.get(protocol::jss::engine_result) {
            tracing::debug!(target: "rpc", result = %engine_result, "Transaction simulated");
        }
    }
    Ok(result)
}
