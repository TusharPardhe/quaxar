//! Tests for the ledger entry helpers RPC handler.

pub(super) use std::collections::BTreeMap;

pub(super) use basics::base_uint::{Uint160, Uint256};
pub(super) use protocol::{
    AccountID, JsonValue, account_keylet, bridge_keylet_from_door_issue, credential_keylet,
    deposit_preauth_credentials_keylet, deposit_preauth_keylet, line, owner_dir_keylet,
    parse_base58_account_id, to_base58, xchain_owned_claim_id_keylet_from_bridge,
    xchain_owned_create_account_claim_id_keylet_from_bridge, xrp_account,
};
pub(super) use sha2::{Digest, Sha512};

pub(super) use rpc::{
    MAX_CREDENTIALS_ARRAY_SIZE,
    ledger_entry_helpers::{expected_field_error, missing_field_error},
    malformed_error, parse_account_id, parse_account_root, parse_asset, parse_bridge,
    parse_bridge_fields, parse_credential, parse_deposit_preauth_account,
    parse_deposit_preauth_credential_array, parse_directory_node, parse_hex_blob, parse_index,
    parse_issue, parse_ledger_hashes, parse_mpt_issuance, parse_mptoken, parse_ripple_state,
    parse_uint32, parse_uint192, parse_uint256, parse_xchain_owned_claim_id,
    parse_xchain_owned_create_account_claim_id, required_account_id, required_asset,
    required_hex_blob, required_issue, required_uint32, required_uint192, required_uint256,
};

pub(super) fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

pub(super) fn error_fields(value: &JsonValue) -> (&str, i64, &str) {
    let JsonValue::Object(object) = value else {
        panic!("expected error object");
    };
    let JsonValue::String(error) = object.get("error").expect("error") else {
        panic!("expected error string");
    };
    let JsonValue::Signed(code) = object.get("error_code").expect("error_code") else {
        panic!("expected error code");
    };
    let JsonValue::String(message) = object.get("error_message").expect("error_message") else {
        panic!("expected error message");
    };
    (error, *code, message)
}

pub(super) fn sha512_half(parts: &[&[u8]]) -> Uint256 {
    let mut hasher = Sha512::new();
    for part in parts {
        hasher.update(part);
    }
    let digest = hasher.finalize();
    Uint256::from_slice(&digest[..32]).expect("SHA-512 half output must contain 32 bytes")
}

mod error_and_parsing;
mod field_validation;
