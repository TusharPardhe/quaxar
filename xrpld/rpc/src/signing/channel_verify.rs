//! Read-only `channel_verify` RPC slice.
//!
//! This keeps the the reference implementation request parsing and payment-channel signature
//! verification shape, using the existing protocol serializer and explicit
//! crypto crates rather than a hidden runtime owner.

use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use basics::string_utilities::{str_unhex, to_uint64};
use protocol::{
    JsonValue, PublicKey, TokenType, parse_base58_with_type, serialize_pay_chan_authorization,
    verify,
};

use crate::commands::rpc_helpers::{missing_field_error, rpc_error};
use crate::status::RpcErrorCode;

fn json_value_as_string(value: &JsonValue) -> String {
    match value {
        JsonValue::String(text) => text.clone(),
        JsonValue::Signed(value) => value.to_string(),
        JsonValue::Unsigned(value) => value.to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => String::new(),
    }
}

fn parse_public_key(text: &str) -> Option<PublicKey> {
    parse_base58_with_type::<PublicKey>(TokenType::AccountPublic, text).or_else(|| {
        let blob = str_unhex(text)?;
        PublicKey::from_slice(&blob).ok()
    })
}

fn parse_required_field(params: &JsonValue, field: &'static str) -> Result<String, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Err(missing_field_error(field));
    };

    let Some(value) = object.get(field) else {
        return Err(missing_field_error(field));
    };

    Ok(json_value_as_string(value))
}

fn parse_amount_field(params: &JsonValue) -> Result<u64, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Err(rpc_error(RpcErrorCode::ChannelAmtMalformed));
    };

    let Some(value) = object.get("amount") else {
        return Err(missing_field_error("amount"));
    };
    let JsonValue::String(text) = value else {
        return Err(rpc_error(RpcErrorCode::ChannelAmtMalformed));
    };

    to_uint64(text).ok_or_else(|| rpc_error(RpcErrorCode::ChannelAmtMalformed))
}

pub fn do_channel_verify(params: &JsonValue) -> JsonValue {
    for field in ["public_key", "channel_id", "amount", "signature"] {
        if matches!(params, JsonValue::Object(object) if !object.contains_key(field)) {
            return missing_field_error(field);
        }
    }

    let public_key_text = match parse_required_field(params, "public_key") {
        Ok(text) => text,
        Err(error) => return error,
    };
    let channel_id_text = match parse_required_field(params, "channel_id") {
        Ok(text) => text,
        Err(error) => return error,
    };
    let signature_text = match parse_required_field(params, "signature") {
        Ok(text) => text,
        Err(error) => return error,
    };

    let Some(public_key) = parse_public_key(&public_key_text) else {
        return rpc_error(RpcErrorCode::PublicMalformed);
    };

    let Ok(channel_id) = Uint256::from_hex(&channel_id_text) else {
        return rpc_error(RpcErrorCode::ChannelMalformed);
    };

    let amount = match parse_amount_field(params) {
        Ok(amount) => amount,
        Err(error) => return error,
    };

    let Some(signature) = str_unhex(&signature_text) else {
        return rpc_error(RpcErrorCode::InvalidParams);
    };
    if signature.is_empty() {
        return rpc_error(RpcErrorCode::InvalidParams);
    }

    let message = serialize_pay_chan_authorization(&channel_id, amount);
    let verified = verify(&public_key, &message, &signature);

    JsonValue::Object(BTreeMap::from([(
        "signature_verified".to_owned(),
        JsonValue::Bool(verified),
    )]))
}
