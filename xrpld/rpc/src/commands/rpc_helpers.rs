//! RPC helper ports from `xrpld/rpc/detail/RPCHelpers.*`.

#![allow(dead_code)]

use std::collections::BTreeMap;

use basics::{base_uint::Uint256, str_hex::str_hex, string_utilities::to_uint64};
use protocol::{
    JsonOptions, JsonValue, KeyType, LedgerEntryType, LedgerFormats, PublicKey, STArray, STObject,
    STParsedJSONObject, STTx, SecretKey, Seed, SerialIter, StBase, build_multi_signing_data,
    derive_public_key, generate_secret_key, get_field_by_name, get_field_by_symbol, jss,
    parse_base58_account_id, serialize_pay_chan_authorization, sf_generic, sign,
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

    let signing_for_id = st_tx.get_fee_payer();
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
        map.insert(jss::Fee.to_string(), JsonValue::String("10".to_string()));
    }
    if !map.contains_key(jss::Sequence) {
        map.insert(jss::Sequence.to_string(), JsonValue::Unsigned(1));
    }
    Ok(())
}

pub fn simulate_txn<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, SimulateSource, Runtime>,
    tx: &STTx,
) -> Result<JsonValue, Status> {
    tracing::debug!(target: "rpc", method = "simulate", "RPC request received");
    let mut ret = BTreeMap::new();
    ret.insert(jss::applied.to_string(), JsonValue::Bool(false));

    // If a ledger is available, run the real transactor and capture metadata
    if let Some(ledger) = ctx.runtime.current_ledger_for_simulation() {
        let ledger_seq = ledger.header().seq;
        let mut view = ledger::ApplyViewImpl::new(ledger, tx::ApplyFlags::NONE);
        let txn_type = tx.get_txn_type();
        let result = app::apply_submit_transactor_shell(&mut view, tx, txn_type);

        ret.insert(
            jss::engine_result.to_string(),
            JsonValue::String(format!("{:?}", result)),
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

        // Build metadata from the view's change table
        let affected_nodes = view.table().to_simulation_metadata();
        let mut meta = BTreeMap::new();
        meta.insert(
            "AffectedNodes".to_string(),
            JsonValue::Array(affected_nodes),
        );
        meta.insert(
            "TransactionResult".to_string(),
            JsonValue::String(format!("{:?}", result)),
        );
        ret.insert("meta".to_string(), JsonValue::Object(meta));
    } else {
        ret.insert(
            jss::engine_result.to_string(),
            JsonValue::String("tesSUCCESS".to_string()),
        );
        ret.insert(jss::engine_result_code.to_string(), JsonValue::Signed(0));
    }

    if !ctx
        .params
        .get(jss::binary)
        .and_then(|v| match v {
            JsonValue::Bool(b) => Some(*b),
            _ => None,
        })
        .unwrap_or(false)
    {
        ret.insert(jss::tx_json.to_string(), tx.json(JsonOptions::new(0)));
    } else {
        ret.insert(
            jss::tx_blob.to_string(),
            JsonValue::String(hex::encode(tx.get_serializer().data())),
        );
    }

    Ok(JsonValue::Object(ret))
}

pub fn parse_xrpl_lib_seed(s: &str) -> Option<Seed> {
    protocol::parse_base58_seed(s)
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
    let parsed = STParsedJSONObject::new("tx_json", tx_json);
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
