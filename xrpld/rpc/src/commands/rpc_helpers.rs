//! RPC helper ports from `xrpld/rpc/detail/RPCHelpers.*`.

#![allow(dead_code)]

use std::{collections::BTreeMap, sync::Arc};

use basics::{base_uint::Uint256, str_hex::str_hex, string_utilities::to_uint64};
use protocol::tokens::decode_base58_token_multibyte;
use protocol::{
    JsonOptions, JsonValue, KeyType, LedgerEntryType, LedgerFormats, PublicKey, STArray, STObject,
    STParsedJSONObject, STTx, SecretKey, Seed, SerialIter, Serializer, StBase,
    build_multi_signing_data, derive_public_key, generate_secret_key, get_field_by_name,
    get_field_by_symbol, is_tec_claim, is_tes_success, jss, parse_base58_account_id,
    serialize_pay_chan_authorization, sf_generic, sign,
};

#[cfg(not(test))]
use crate::simulate::SimulateSource;
#[cfg(not(test))]
use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::state::role::{Role, is_unlimited};
use crate::state::tuning::LimitRange;
use crate::status::{RpcErrorCode, Status};
use crate::{insert_deliver_max, key_type_from_string};
#[cfg(test)]
use rpc::context::{RpcRequestContext, RpcRuntime};
#[cfg(test)]
use rpc::simulate::SimulateSource;

pub fn inject_error(code: RpcErrorCode, json: &mut JsonValue) {
    tracing::warn!(target: "rpc", error = ?code, "RPC request failed");
    Status::new(code).inject(json);
}

pub fn inject_error_message(code: RpcErrorCode, message: impl Into<String>, json: &mut JsonValue) {
    tracing::warn!(target: "rpc", error = ?code, "RPC request failed");
    Status::with_message(code, message).inject(json);
}

pub fn make_error(code: RpcErrorCode) -> JsonValue {
    let mut json = JsonValue::Object(BTreeMap::new());
    inject_error(code, &mut json);
    json
}

pub fn make_error_message(code: RpcErrorCode, message: impl Into<String>) -> JsonValue {
    let mut json = JsonValue::Object(BTreeMap::new());
    inject_error_message(code, message, &mut json);
    json
}

pub fn rpc_error(code: RpcErrorCode) -> JsonValue {
    make_error(code)
}

pub fn expected_field_message(name: impl AsRef<str>, ty: impl AsRef<str>) -> String {
    Status::expected_field_message(name, ty)
}

pub fn expected_field_error(name: impl AsRef<str>, ty: impl AsRef<str>) -> JsonValue {
    make_error_message(
        RpcErrorCode::InvalidParams,
        Status::expected_field_message(name, ty),
    )
}

pub fn object_field_error(name: impl AsRef<str>) -> JsonValue {
    expected_field_error(name, "object")
}

pub fn missing_field_message(name: impl AsRef<str>) -> String {
    Status::missing_field_message(name)
}

pub fn missing_field_error(name: impl AsRef<str>) -> JsonValue {
    make_error_message(
        RpcErrorCode::InvalidParams,
        Status::missing_field_message(name),
    )
}

pub fn invalid_field_message(name: impl AsRef<str>) -> String {
    Status::invalid_field_message(name)
}

pub fn invalid_field_error(name: impl AsRef<str>) -> JsonValue {
    make_error_message(
        RpcErrorCode::InvalidParams,
        Status::invalid_field_message(name),
    )
}

pub fn transaction_sign<Runtime: RpcRuntime, Source>(
    ctx: &RpcRequestContext<'_, Source, Runtime>,
) -> Result<JsonValue, Status> {
    tracing::debug!(target: "rpc", method = "sign", "RPC request received");
    let JsonValue::Object(params) = &ctx.params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    let tx_json = params.get(jss::tx_json).cloned().ok_or_else(|| {
        Status::with_message(
            RpcErrorCode::InvalidParams,
            missing_field_message(jss::tx_json),
        )
    })?;
    let mut st_tx = parse_sttx_from_json_value(&tx_json)?;
    // Match rippled's autofill: always set tfFullyCanonicalSig
    st_tx.set_flag(0x8000_0000);
    // Auto-fill NetworkID for private networks (ID > 1024)
    let app_network_id = ctx.runtime.app().map(|app| app.network_id()).unwrap_or(0);
    if app_network_id > 1024 && !st_tx.is_field_present(get_field_by_symbol("sfNetworkID")) {
        st_tx.set_field_u32(get_field_by_symbol("sfNetworkID"), app_network_id);
    }
    // Auto-fill Sequence from the current account state (matching rippled).
    // If Sequence is 0 (default/missing), look up the account's current sequence.
    // Use the open ledger state (which reflects submitted-but-not-yet-closed txs)
    // so that multiple transactions in the same ledger get correct sequences.
    if st_tx.get_seq_value() == 0 {
        if let Some(app) = ctx.runtime.app() {
            let account = st_tx.get_account_id(get_field_by_symbol("sfAccount"));
            let account_keylet =
                protocol::account_keylet(basics::base_uint::Uint160::from_void(account.data()));
            // Try open ledger first (has latest state including pending txs)
            let seq = app
                .network_ops_current_account_seq(&account)
                .or_else(|| {
                    app.closed_ledger()
                        .or_else(|| app.validated_ledger())
                        .and_then(|ledger| ledger.read(account_keylet).ok().flatten())
                        .map(|sle| sle.get_field_u32(get_field_by_symbol("sfSequence")))
                })
                .unwrap_or(1);
            st_tx.set_field_u32(get_field_by_symbol("sfSequence"), seq);
        }
    }
    // Auto-fill Fee if not set (default to base fee)
    if !st_tx.is_field_present(get_field_by_symbol("sfFee"))
        || st_tx
            .get_field_amount(get_field_by_symbol("sfFee"))
            .xrp()
            .drops()
            == 0
    {
        st_tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            protocol::STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(12)),
        );
    }
    let (public_key, secret_key) = keypair_for_signature(params)?;
    let signature_target = parse_signature_target(params)?;
    signer_target_object(&mut st_tx, signature_target).set_field_vl(
        get_field_by_symbol("sfSigningPubKey"),
        public_key.as_bytes(),
    );
    st_tx
        .sign(&public_key, &secret_key, signature_target)
        .map_err(|_| Status::new(RpcErrorCode::Internal))?;

    let mut result = transaction_format_result(&st_tx, ctx.api_version);
    result.insert(
        jss::deprecated.to_string(),
        JsonValue::String(
            "This command has been deprecated and will be removed in a future version of the server. Please migrate to a standalone signing tool.".to_owned(),
        ),
    );
    Ok(JsonValue::Object(result))
}

pub fn transaction_sign_for<Runtime: RpcRuntime, Source>(
    ctx: &RpcRequestContext<'_, Source, Runtime>,
) -> Result<JsonValue, Status> {
    tracing::debug!(target: "rpc", method = "sign_for", "RPC request received");
    let JsonValue::Object(params) = &ctx.params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    let signer_account = params
        .get("account")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            Status::with_message(
                RpcErrorCode::InvalidParams,
                missing_field_message("account"),
            )
        })?;
    let signer_account_id = parse_base58_account_id(signer_account).ok_or_else(|| {
        Status::with_message(
            RpcErrorCode::InvalidParams,
            invalid_field_message("account"),
        )
    })?;

    let mut tx_json = params.get(jss::tx_json).cloned().ok_or_else(|| {
        Status::with_message(
            RpcErrorCode::InvalidParams,
            missing_field_message(jss::tx_json),
        )
    })?;
    let JsonValue::Object(ref mut tx_object) = tx_json else {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            expected_field_message(jss::tx_json, "object"),
        ));
    };
    let app_network_id = ctx.runtime.app().map(|app| app.network_id()).unwrap_or(0);
    // Auto-fill NetworkID for private networks (ID > 1024), matching rippled's autofill
    if app_network_id > 1024 && !tx_object.contains_key(jss::NetworkID) {
        tx_object.insert(
            jss::NetworkID.to_owned(),
            JsonValue::Unsigned(u64::from(app_network_id)),
        );
    }
    check_transaction_sign_for_network_id(tx_object, app_network_id)?;

    tx_object
        .entry(jss::SigningPubKey.to_owned())
        .or_insert_with(|| JsonValue::String(String::new()));

    if !tx_object.contains_key(jss::Sequence) {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            missing_field_message("tx_json.Sequence"),
        ));
    }
    if !params.contains_key(jss::signature_target)
        && tx_object
            .get(jss::SigningPubKey)
            .and_then(JsonValue::as_str)
            .is_some_and(|value| !value.is_empty())
    {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            "When multi-signing 'tx_json.SigningPubKey' must be empty.",
        ));
    }

    let mut st_tx = parse_sttx_from_json_value(&tx_json)?;
    let (public_key, secret_key) = keypair_for_signature(params)?;
    let signature_target = parse_signature_target(params)?;

    let signing_data = build_multi_signing_data(&st_tx.clone_as_object(), signer_account_id);
    let signature = sign(&public_key, &secret_key, signing_data.data())
        .map_err(|_| Status::new(RpcErrorCode::Internal))?;

    let signing_for_id = st_tx.get_initiator();
    let mut signers = signer_target_object(&mut st_tx, signature_target)
        .get_field_array(get_field_by_symbol("sfSigners"))
        .iter()
        .cloned()
        .collect::<Vec<_>>();

    let mut signer = STObject::make_inner_object(get_field_by_symbol("sfSigner"));
    signer.set_account_id(get_field_by_symbol("sfAccount"), signer_account_id);
    signer.set_field_vl(
        get_field_by_symbol("sfSigningPubKey"),
        public_key.as_bytes(),
    );
    signer.set_field_vl(get_field_by_symbol("sfTxnSignature"), &signature);
    signers.push(signer);
    signers.sort_by_key(|entry| entry.get_account_id(get_field_by_symbol("sfAccount")));

    if signers.windows(2).any(|pair| {
        pair[0].get_account_id(get_field_by_symbol("sfAccount"))
            == pair[1].get_account_id(get_field_by_symbol("sfAccount"))
    }) {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            "Duplicate Signers:Signer:Account entries are not allowed.",
        ));
    }
    if signers
        .iter()
        .any(|entry| entry.get_account_id(get_field_by_symbol("sfAccount")) == signing_for_id)
    {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            format!(
                "A Signer may not be the transaction's Account ({}).",
                protocol::to_base58(signing_for_id)
            ),
        ));
    }

    let mut signers_array = STArray::new(get_field_by_symbol("sfSigners"));
    for signer in signers {
        signers_array.push_back(signer);
    }
    signer_target_object(&mut st_tx, signature_target)
        .set_field_array(get_field_by_symbol("sfSigners"), signers_array);

    let mut result = transaction_format_result(&st_tx, ctx.api_version);
    result.insert(
        jss::deprecated.to_string(),
        JsonValue::String(
            "This command has been deprecated and will be removed in a future version of the server. Please migrate to a standalone signing tool.".to_owned(),
        ),
    );
    Ok(JsonValue::Object(result))
}

fn check_transaction_sign_for_network_id(
    tx_object: &BTreeMap<String, JsonValue>,
    app_network_id: u32,
) -> Result<(), Status> {
    if app_network_id <= 1024 {
        return Ok(());
    }

    let Some(network_id) = tx_object.get(jss::NetworkID) else {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            missing_field_message("tx_json.NetworkID"),
        ));
    };

    if network_id.as_u64() != Some(u64::from(app_network_id)) {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            invalid_field_message("tx_json.NetworkID"),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod transaction_sign_for_network_id_tests {
    use super::*;

    #[test]
    fn network_id_is_required_and_must_match_for_networks_above_1024() {
        let empty = BTreeMap::new();
        assert_eq!(
            check_transaction_sign_for_network_id(&empty, 21338),
            Err(Status::with_message(
                RpcErrorCode::InvalidParams,
                missing_field_message("tx_json.NetworkID")
            ))
        );

        let wrong = BTreeMap::from([(jss::NetworkID.to_owned(), JsonValue::Unsigned(21337))]);
        assert_eq!(
            check_transaction_sign_for_network_id(&wrong, 21338),
            Err(Status::with_message(
                RpcErrorCode::InvalidParams,
                invalid_field_message("tx_json.NetworkID")
            ))
        );

        let correct = BTreeMap::from([(jss::NetworkID.to_owned(), JsonValue::Unsigned(21338))]);
        assert!(check_transaction_sign_for_network_id(&correct, 21338).is_ok());
    }
}

pub fn get_tx_json_from_params(params: &JsonValue) -> Result<JsonValue, Status> {
    let JsonValue::Object(map) = params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    if let Some(blob) = map.get(jss::tx_blob) {
        if map.contains_key(jss::tx_json) {
            return Err(Status::with_message(
                RpcErrorCode::InvalidParams,
                "Can only include one of `tx_blob` and `tx_json`.",
            ));
        }
        let JsonValue::String(hex_str) = blob else {
            return Err(Status::new(RpcErrorCode::InvalidParams));
        };
        let bytes = hex::decode(hex_str).map_err(|_| Status::new(RpcErrorCode::InvalidParams))?;
        let mut iter = SerialIter::new(&bytes);
        let obj = STObject::from_serial_iter(&mut iter, sf_generic(), 0);
        Ok(obj.json(JsonOptions::new(0)))
    } else if let Some(tx_json) = map.get(jss::tx_json) {
        if !matches!(tx_json, JsonValue::Object(_)) {
            return Err(Status::new(RpcErrorCode::InvalidParams));
        }
        Ok(tx_json.clone())
    } else {
        Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            "Neither `tx_blob` nor `tx_json` included.",
        ))
    }
}

pub fn parse_sttx_from_params(params: &JsonValue) -> Result<STTx, Status> {
    let JsonValue::Object(map) = params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    if map.contains_key(jss::tx_blob) && map.contains_key(jss::tx_json) {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            "Can only include one of `tx_blob` and `tx_json`.",
        ));
    }

    if let Some(blob) = map.get(jss::tx_blob) {
        let JsonValue::String(hex_str) = blob else {
            return Err(Status::new(RpcErrorCode::InvalidParams));
        };
        let bytes = hex::decode(hex_str).map_err(|_| Status::new(RpcErrorCode::InvalidParams))?;
        let mut iter = SerialIter::new(&bytes);
        return std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            STTx::from_serial_iter(&mut iter)
        }))
        .map_err(|_| Status::new(RpcErrorCode::InvalidParams));
    }

    parse_sttx_from_json_value(&get_tx_json_from_params(params)?)
}

pub fn channel_authorize<Runtime: RpcRuntime, Source>(
    ctx: &RpcRequestContext<'_, Source, Runtime>,
) -> Result<JsonValue, Status> {
    tracing::debug!(target: "rpc", method = "channel_authorize", "RPC request received");
    let JsonValue::Object(params) = &ctx.params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    if !params.contains_key(jss::channel_id) {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            missing_field_message(jss::channel_id),
        ));
    }
    if !params.contains_key(jss::amount) {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            missing_field_message(jss::amount),
        ));
    }
    if !params.contains_key(jss::key_type) && !params.contains_key(jss::secret) {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            missing_field_message(jss::secret),
        ));
    }

    let channel_id = params
        .get(jss::channel_id)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Status::new(RpcErrorCode::ChannelMalformed))?;
    let amount = params
        .get(jss::amount)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Status::new(RpcErrorCode::ChannelAmtMalformed))?;

    let (public_key, secret_key) = keypair_for_signature(params)?;
    let channel_id =
        Uint256::from_hex(channel_id).map_err(|_| Status::new(RpcErrorCode::ChannelMalformed))?;
    let amount = to_uint64(amount).ok_or_else(|| Status::new(RpcErrorCode::ChannelAmtMalformed))?;
    let signature = sign(
        &public_key,
        &secret_key,
        &serialize_pay_chan_authorization(&channel_id, amount),
    )
    .map_err(|_| Status::new(RpcErrorCode::Internal))?;

    let mut ret = BTreeMap::new();
    ret.insert(
        jss::signature.to_string(),
        JsonValue::String(str_hex(&signature)),
    );
    Ok(JsonValue::Object(ret))
}

pub fn autofill_tx<Runtime: RpcRuntime>(
    tx_json: &mut protocol::JsonValue,
    _ctx: &RpcRequestContext<'_, SimulateSource, Runtime>,
) -> Result<(), Status> {
    let JsonValue::Object(map) = tx_json else {
        return Ok(());
    };
    if !map.contains_key(jss::Fee) {
        // Parity: ../rippled/src/xrpld/rpc/handlers/transaction/Simulate.cpp::
        // simulateTxn copies OpenLedger and uses TxQ's live fee context.
        let fee = _ctx
            .runtime
            .app()
            .map(|app| app.open_ledger().current().base_fee_drops)
            .unwrap_or(10);
        map.insert(jss::Fee.to_string(), JsonValue::String(fee.to_string()));
    }
    if !map.contains_key(jss::Sequence) {
        let seq = _ctx
            .runtime
            .app()
            .and_then(|app| match map.get("Account") {
                Some(JsonValue::String(account)) => protocol::parse_base58_account_id(account)
                    .and_then(|account| app.network_ops_current_account_seq(&account))
                    .map(u64::from),
                _ => None,
            })
            .unwrap_or(1);
        map.insert(jss::Sequence.to_string(), JsonValue::Unsigned(seq));
    }
    Ok(())
}

pub fn simulate_txn<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, SimulateSource, Runtime>,
    tx: &STTx,
) -> Result<JsonValue, Status> {
    tracing::debug!(target: "rpc", method = "simulate", "RPC request received");
    let mut ret = BTreeMap::new();
    let binary = ctx
        .params
        .get(jss::binary)
        .and_then(|value| match value {
            JsonValue::Bool(value) => Some(*value),
            _ => None,
        })
        .unwrap_or(false);
    let mut simulation_meta_blob = None;
    ret.insert(jss::applied.to_string(), JsonValue::Bool(false));

    // The real application owns TxQ and its open-ledger/fee state. Route
    // simulation through that canonical admission boundary instead of calling
    // the transactor shell directly. Parity: ../rippled/src/xrpld/rpc/handlers/
    // transaction/Simulate.cpp::simulateTxn invokes TxQ::apply(TapDryRun).
    if let Some(app) = ctx.runtime.app() {
        // ApplicationRoot::simulate_transaction clones its persistent OpenView
        // sandbox. The runtime ledger is only the immutable fallback for a
        // node that has not accepted any open-ledger mutations yet; rebuilding
        // `Ledger::from_previous` here would discard sequence/balance changes.
        // Parity: ../rippled/src/xrpld/rpc/handlers/transaction/Simulate.cpp::
        // simulateTxn copies OpenLedger::current() before TxQ::apply(TapDryRun).
        let ledger = ctx
            .runtime
            .current_ledger_for_simulation()
            .ok_or_else(|| Status::new(RpcErrorCode::NotSynced))?;
        let outcome = app.simulate_transaction(ledger, Arc::new(tx.clone()));
        ret.insert(
            jss::applied.to_string(),
            JsonValue::Bool(outcome.result.applied),
        );
        ret.insert(
            jss::engine_result.to_string(),
            JsonValue::String(protocol::trans_token(outcome.result.ter).to_owned()),
        );
        ret.insert(
            jss::engine_result_code.to_string(),
            JsonValue::Signed(outcome.result.ter.to_int() as i64),
        );
        ret.insert(
            "engine_result_message".to_string(),
            JsonValue::String(protocol::trans_human(outcome.result.ter).to_owned()),
        );
        ret.insert(
            jss::ledger_index.to_string(),
            JsonValue::Unsigned(u64::from(outcome.ledger_seq)),
        );

        if let Some(mut metadata) = outcome.metadata {
            if binary {
                let mut serializer = Serializer::default();
                metadata.add_raw(&mut serializer, outcome.result.ter, 0);
                ret.insert(
                    "meta_blob".to_string(),
                    JsonValue::String(hex::encode(serializer.data())),
                );
            } else {
                let mut meta = metadata.get_json(JsonOptions::new(0));
                if is_tes_success(outcome.result.ter) {
                    crate::handlers::delivered_amount::insert_delivered_amount(
                        &mut meta,
                        outcome.ledger_seq,
                        Some(outcome.close_time),
                        tx,
                        &metadata,
                    );
                }
                ret.insert("meta".to_string(), meta);
            }
        } else {
            ret.insert(
                "meta".to_string(),
                JsonValue::Object(BTreeMap::from([
                    ("AffectedNodes".to_owned(), JsonValue::Array(Vec::new())),
                    (
                        "TransactionResult".to_owned(),
                        JsonValue::String(protocol::trans_token(outcome.result.ter).to_owned()),
                    ),
                ])),
            );
        }

        if binary {
            ret.insert(
                jss::tx_blob.to_string(),
                JsonValue::String(hex::encode(tx.get_serializer().data())),
            );
        } else {
            ret.insert(jss::tx_json.to_string(), tx.json(JsonOptions::new(0)));
        }
        return Ok(JsonValue::Object(ret));
    }

    // Keep the generic fallback for lightweight RPC runtimes that do not own
    // an ApplicationRoot/TxQ (unit harnesses and isolated handler tests).
    if let Some(ledger) = ctx.runtime.current_ledger_for_simulation() {
        let ledger_seq = ledger.header().seq;
        let close_time = ledger.header().close_time;
        let mut view = ledger::ApplyViewImpl::new(Arc::clone(&ledger), tx::ApplyFlags::NONE);
        // Minimal RPC runtimes have no ApplicationRoot/TxQ owner. Keep their
        // fallback on the app-level canonical dry-run boundary.
        let (result, delivered_amount) =
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                app::apply_simulated_transaction(&mut view, tx)
            })) {
                Ok((result, delivered_amount)) => (result, delivered_amount),
                Err(_) => {
                    ret.insert(
                        jss::engine_result.to_string(),
                        JsonValue::String("telLOCAL_ERROR".to_string()),
                    );
                    ret.insert(jss::engine_result_code.to_string(), JsonValue::Signed(-399));
                    return Ok(JsonValue::Object(ret));
                }
            };

        ret.insert(
            jss::engine_result.to_string(),
            JsonValue::String(protocol::trans_token(result).to_owned()),
        );
        ret.insert(
            jss::engine_result_code.to_string(),
            JsonValue::Signed(result.to_int() as i64),
        );
        ret.insert(
            "engine_result_message".to_string(),
            JsonValue::String(protocol::trans_human(result).to_string()),
        );
        ret.insert(
            jss::ledger_index.to_string(),
            JsonValue::Unsigned(u64::from(ledger_seq)),
        );

        // Build one typed metadata object from the actual dry-run changes.
        // JSON and binary simulation responses must render the same metadata.
        if is_tes_success(result) || protocol::is_tec_claim(result) {
            let mut transaction_meta =
                view.table()
                    .to_tx_meta(tx.get_transaction_id(), ledger_seq, delivered_amount);
            let mut serializer = Serializer::default();
            transaction_meta.add_raw(&mut serializer, result, 0);

            let mut meta = transaction_meta.get_json(JsonOptions::new(0));
            if is_tes_success(result) {
                crate::handlers::delivered_amount::insert_delivered_amount(
                    &mut meta,
                    ledger_seq,
                    Some(close_time),
                    tx,
                    &transaction_meta,
                );
            }
            simulation_meta_blob = Some(hex::encode(serializer.data()));
            ret.insert("meta".to_string(), meta);
        } else {
            let mut meta = BTreeMap::new();
            meta.insert("AffectedNodes".to_owned(), JsonValue::Array(Vec::new()));
            meta.insert(
                "TransactionResult".to_owned(),
                JsonValue::String(format!("{:?}", result)),
            );
            ret.insert("meta".to_string(), JsonValue::Object(meta));
        }
    } else {
        ret.insert(
            jss::engine_result.to_string(),
            JsonValue::String("tesSUCCESS".to_string()),
        );
        ret.insert(jss::engine_result_code.to_string(), JsonValue::Signed(0));
        ret.insert(
            "engine_result_message".to_string(),
            JsonValue::String(
                "The transaction was applied. Only final in a validated ledger.".to_string(),
            ),
        );
    }

    if !binary {
        ret.insert(jss::tx_json.to_string(), tx.json(JsonOptions::new(0)));
    } else {
        ret.insert(
            jss::tx_blob.to_string(),
            JsonValue::String(hex::encode(tx.get_serializer().data())),
        );
        if let Some(meta_blob) = simulation_meta_blob {
            ret.insert("meta_blob".to_string(), JsonValue::String(meta_blob));
        }
    }

    Ok(JsonValue::Object(ret))
}

/// Decode only xrpl.js's legacy Ed25519 seed encoding.
///
/// rippled's `parseXrplLibSeed` accepts the raw Base58 payload prefix E1 4B,
/// followed by exactly 16 seed bytes. A normal XRPL family seed must not be
/// accepted here: it falls through to normal seed handling and therefore
/// retains the default Secp256k1 key type.
pub fn parse_xrpl_lib_seed(s: &str) -> Option<Seed> {
    decode_base58_token_multibyte(s, &[0xE1, 0x4B])
        .filter(|bytes| bytes.len() == 16)
        .and_then(|bytes| Seed::from_slice(&bytes).ok())
}

pub fn get_seed_from_rpc(params: &JsonValue) -> Result<Seed, Status> {
    let JsonValue::Object(map) = params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    if let Some(JsonValue::String(passphrase)) = map.get(jss::passphrase) {
        return Ok(protocol::generate_seed(passphrase));
    }

    if let Some(JsonValue::String(seed)) = map.get(jss::seed) {
        return protocol::parse_generic_seed(seed, false)
            .ok_or_else(|| Status::new(RpcErrorCode::InvalidParams));
    }

    if let Some(JsonValue::String(seed_hex)) = map.get(jss::seed_hex) {
        let bytes = hex::decode(seed_hex).map_err(|_| Status::new(RpcErrorCode::InvalidParams))?;
        return Seed::from_slice(&bytes).map_err(|_| Status::new(RpcErrorCode::InvalidParams));
    }

    Err(Status::new(RpcErrorCode::InvalidParams))
}

pub fn read_limit_field(params: &JsonValue, role: Role, range: LimitRange) -> Result<u32, Status> {
    let JsonValue::Object(object) = params else {
        return Ok(range.r_default);
    };

    let Some(limit_value) = object.get("limit") else {
        return Ok(range.r_default);
    };

    if matches!(limit_value, JsonValue::Null) {
        return Ok(range.r_default);
    }

    let mut limit = match limit_value {
        JsonValue::Unsigned(value) => u32::try_from(*value)
            .map_err(|_| Status::expected_field_error("limit", "unsigned integer"))?,
        JsonValue::Signed(value) if *value >= 0 => u32::try_from(*value as u64)
            .map_err(|_| Status::expected_field_error("limit", "unsigned integer"))?,
        _ => return Err(Status::expected_field_error("limit", "unsigned integer")),
    };

    if limit == 0 {
        return Err(Status::invalid_field_error("limit"));
    }

    if !is_unlimited(role) {
        limit = limit.clamp(range.rmin, range.rmax);
    }

    Ok(limit)
}

pub fn read_limit_field_with_cap(
    params: &JsonValue,
    role: Role,
    default_limit: u32,
    cap: u32,
) -> Result<u32, Status> {
    let JsonValue::Object(object) = params else {
        return Ok(default_limit);
    };

    let Some(limit_value) = object.get("limit") else {
        return Ok(default_limit);
    };

    if matches!(limit_value, JsonValue::Null) {
        return Ok(default_limit);
    }

    let mut limit = match limit_value {
        JsonValue::Unsigned(value) => u32::try_from(*value)
            .map_err(|_| Status::expected_field_error("limit", "unsigned integer"))?,
        JsonValue::Signed(value) if *value >= 0 => u32::try_from(*value as u64)
            .map_err(|_| Status::expected_field_error("limit", "unsigned integer"))?,
        _ => return Err(Status::expected_field_error("limit", "unsigned integer")),
    };

    if limit == 0 {
        return Err(Status::invalid_field_error("limit"));
    }

    if !is_unlimited(role) {
        limit = limit.min(cap);
    }

    Ok(limit)
}

pub fn choose_ledger_entry_type(params: &JsonValue) -> Result<Option<LedgerEntryType>, Status> {
    let JsonValue::Object(object) = params else {
        return Ok(None);
    };

    let Some(type_value) = object.get("type") else {
        return Ok(None);
    };

    let JsonValue::String(filter) = type_value else {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            "Invalid field 'type', not string.",
        ));
    };

    LedgerFormats::get_instance()
        .iter()
        .find_map(|item| {
            let canonical_match = item.name().eq_ignore_ascii_case(filter);
            let rpc_match = item.metadata().rpc_name == filter;
            (canonical_match || rpc_match).then(|| item.format_type())
        })
        .map(Some)
        .ok_or_else(|| Status::with_message(RpcErrorCode::InvalidParams, "Invalid field 'type'."))
}

fn parse_sttx_from_json_value(tx_json: &JsonValue) -> Result<STTx, Status> {
    // Strip display-only / computed fields that are not protocol SFields.
    // sign_for and tx results include these in their tx_json output, but
    // they must be removed before re-parsing for submit_multisigned.
    let cleaned = match tx_json {
        JsonValue::Object(map) => {
            let mut m = map.clone();
            m.remove(jss::DeliverMax);
            m.remove(jss::hash);
            JsonValue::Object(m)
        }
        other => other.clone(),
    };

    let parsed = STParsedJSONObject::new("tx_json", &cleaned);
    if let Some(object) = parsed.object {
        return Ok(STTx::from_stobject(object));
    }

    let mut status = Status::new(RpcErrorCode::InvalidParams);
    if let JsonValue::Object(mut error_map) = parsed.error {
        if let Some(JsonValue::String(message)) = error_map.remove("error_message") {
            status = Status::with_message(RpcErrorCode::InvalidParams, message);
        }
    }
    Err(status)
}

fn keypair_for_signature(
    params: &BTreeMap<String, JsonValue>,
) -> Result<(PublicKey, SecretKey), Status> {
    let mut key_type = match params.get(jss::key_type) {
        Some(JsonValue::String(value)) => Some(
            key_type_from_string(value).ok_or_else(|| Status::new(RpcErrorCode::InvalidParams))?,
        ),
        Some(_) => {
            return Err(Status::with_message(
                RpcErrorCode::InvalidParams,
                expected_field_message(jss::key_type, "string"),
            ));
        }
        None => None,
    };

    // Reject conflicting key sources: only one of secret, seed, seed_hex, or
    // passphrase may be provided.
    let has_secret = params.contains_key(jss::secret);
    let has_seed = params.contains_key(jss::seed);
    let has_seed_hex = params.contains_key(jss::seed_hex);
    let has_passphrase = params.contains_key(jss::passphrase);
    let source_count =
        has_secret as u8 + has_seed as u8 + has_seed_hex as u8 + has_passphrase as u8;
    if source_count > 1 {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            "Exactly one of the following must be specified: secret, seed, seed_hex, passphrase.",
        ));
    }

    let seed = if let Some(JsonValue::String(secret)) = params.get(jss::secret) {
        if key_type.is_none() && secret.starts_with("sEd") {
            key_type = Some(KeyType::Ed25519);
        }
        protocol::parse_generic_seed(secret, false)
            .ok_or_else(|| Status::new(RpcErrorCode::InvalidParams))?
    } else {
        if key_type.is_none() {
            if let Some(JsonValue::String(seed_val)) = params.get(jss::seed) {
                if seed_val.starts_with("sEd") {
                    key_type = Some(KeyType::Ed25519);
                }
            }
        }
        get_seed_from_rpc(&JsonValue::Object(params.clone()))?
    };

    let key_type = key_type.unwrap_or(KeyType::Secp256k1);

    let secret_key =
        generate_secret_key(key_type, &seed).map_err(|_| Status::new(RpcErrorCode::Internal))?;
    let public_key = derive_public_key(key_type, &secret_key)
        .map_err(|_| Status::new(RpcErrorCode::Internal))?;
    Ok((public_key, secret_key))
}

fn parse_signature_target(
    params: &BTreeMap<String, JsonValue>,
) -> Result<Option<&'static protocol::SField>, Status> {
    let Some(value) = params.get(jss::signature_target) else {
        return Ok(None);
    };
    let JsonValue::String(target_name) = value else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };
    let field = get_field_by_name(target_name);
    if field.is_invalid() {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    }
    Ok(Some(field))
}

fn signer_target_object<'a>(
    st_tx: &'a mut STTx,
    signature_target: Option<&'static protocol::SField>,
) -> &'a mut STObject {
    if let Some(target) = signature_target {
        st_tx.peek_field_object(target)
    } else {
        st_tx
    }
}

fn transaction_format_result(st_tx: &STTx, api_version: u32) -> BTreeMap<String, JsonValue> {
    let mut tx_json = if api_version > 1 {
        st_tx.json(JsonOptions::DISABLE_API_PRIOR_V2)
    } else {
        st_tx.json(JsonOptions::NONE)
    };
    insert_deliver_max(&mut tx_json, st_tx.get_txn_type(), api_version);

    let mut result = BTreeMap::new();
    result.insert(jss::tx_json.to_string(), tx_json);
    if api_version > 1 {
        result.insert(
            jss::hash.to_string(),
            JsonValue::String(st_tx.get_transaction_id().to_string()),
        );
    }
    result.insert(
        jss::tx_blob.to_string(),
        JsonValue::String(hex::encode(st_tx.get_serializer().data())),
    );
    result
}
