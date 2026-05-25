//! Read-only `random` RPC slice.
//!
//! This matches the the reference implementation surface by returning a freshly filled
//! `uint256` string and mapping any OS RNG failure to the standard internal
//! RPC error.

use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::JsonValue;
use rand::{RngCore, rngs::OsRng};

use crate::commands::rpc_helpers::rpc_error;
use crate::status::RpcErrorCode;

pub fn do_random() -> JsonValue {
    let mut bytes = [0u8; Uint256::BYTES];
    if OsRng.try_fill_bytes(&mut bytes).is_err() {
        return rpc_error(RpcErrorCode::Internal);
    }

    let value = Uint256::from_array(bytes);
    JsonValue::Object(BTreeMap::from([(
        "random".to_owned(),
        JsonValue::String(value.to_string()),
    )]))
}
