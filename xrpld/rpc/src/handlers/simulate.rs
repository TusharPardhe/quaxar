//! `simulate` handler port from `xrpld/rpc/handlers/transaction/the reference source`.

use crate::commands::rpc_helpers::{
    autofill_tx, get_tx_json_from_params, parse_sttx_from_params, simulate_txn,
};
use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::state::role::Role;
use crate::status::{RpcErrorCode, Status};
use protocol::{JsonValue, STTx, TxType, get_field_by_symbol};

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

    // These fields mean the transaction may run WASM, which simulate cannot
    // charge for. Only admins may run it. Check the parsed transaction shape
    // before the engine runs.
    validate_wasm_simulation(&st_tx, ctx.role)?;

    let result = simulate_txn(ctx, &st_tx)?;
    if let JsonValue::Object(ref obj) = result {
        if let Some(JsonValue::String(engine_result)) = obj.get(protocol::jss::engine_result) {
            tracing::debug!(target: "rpc", result = %engine_result, "Transaction simulated");
        }
    }
    Ok(result)
}

fn wasm_transaction_requires_admin(st_tx: &STTx) -> bool {
    st_tx.is_field_present(get_field_by_symbol("sfBytecode"))
        || st_tx.is_field_present(get_field_by_symbol("sfGas"))
}

fn validate_wasm_simulation(st_tx: &STTx, role: Role) -> Result<(), Status> {
    if wasm_transaction_requires_admin(st_tx) && role != Role::Admin {
        return Err(Status::with_message(
            RpcErrorCode::NoPermission,
            "WASM transactions are only supported in admin mode.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_wasm_simulation, wasm_transaction_requires_admin};
    use crate::state::role::Role;
    use protocol::{JsonValue, STTx, TxType, get_field_by_symbol};

    #[test]
    fn wasm_shape_detects_bytecode_and_gas_fields() {
        let bytecode = STTx::new(TxType::ESCROW_CREATE, |tx| {
            tx.set_field_vl(get_field_by_symbol("sfBytecode"), &[0]);
        });
        let gas = STTx::new(TxType::ESCROW_FINISH, |tx| {
            tx.set_field_u32(get_field_by_symbol("sfGas"), 1);
        });
        let ordinary = STTx::new(TxType::ESCROW_CREATE, |_| {});

        assert!(wasm_transaction_requires_admin(&bytecode));
        assert!(wasm_transaction_requires_admin(&gas));
        assert!(!wasm_transaction_requires_admin(&ordinary));

        let err = validate_wasm_simulation(&bytecode, Role::User).expect_err("user WASM blocked");
        let mut json = JsonValue::Object(Default::default());
        err.inject(&mut json);
        let JsonValue::Object(json) = json else {
            panic!("status must inject an object");
        };
        assert_eq!(json.get("error"), Some(&JsonValue::String("noPermission".to_owned())));
        assert_eq!(
            json.get("error_message"),
            Some(&JsonValue::String(
                "WASM transactions are only supported in admin mode.".to_owned()
            ))
        );
        assert!(validate_wasm_simulation(&bytecode, Role::Admin).is_ok());
        assert!(validate_wasm_simulation(&ordinary, Role::User).is_ok());
    }
}
