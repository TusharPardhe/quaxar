//! Shared parsing and error helpers for the ledger-entry RPC surface.
//!
//! This stays narrower than the full reference `LedgerEntryHelpers.h` port. It
//! covers the reusable JSON/error shaping and the selector helpers that can be
//! expressed with the current Rust protocol surface.

#![allow(clippy::collapsible_if, dead_code, unused_imports)]

use std::collections::BTreeMap;
use std::sync::Arc;

use basics::{
    base_uint::{Uint160, Uint192, Uint256},
    blob::Blob,
    string_utilities::str_unhex,
};
use protocol::{
    AccountID, Asset, Bridge, Currency, Issue, JsonOptions, JsonValue, Keylet, LedgerEntryType,
    STLedgerEntry, StBase, XChainOwnedClaimID, XChainOwnedCreateAccountClaimID, account_keylet,
    amendments_key, asset_from_json, bridge_keylet_from_door_issue, credential_keylet,
    delegate_keylet, deposit_preauth_credentials_keylet, deposit_preauth_keylet, did_keylet,
    escrow_keylet, fee_settings_keylet, issue_from_json, line, loan_broker_keylet, loan_keylet,
    mpt_issuance_keylet_from_mptid, mptoken_keylet_from_mptid, negative_unl_keylet, offer_keylet,
    oracle_keylet, owner_dir_keylet, page_keylet, parse_base58_account_id,
    permissioned_domain_keylet, sha512_half_slices, skip_keylet, skip_keylet_for_ledger,
    ticket_keylet, to_currency, vault_keylet, xchain_owned_claim_id_keylet_from_bridge,
    xchain_owned_create_account_claim_id_keylet_from_bridge,
};

use crate::status::{RpcErrorCode, Status as RpcStatus};

pub const MAX_CREDENTIAL_TYPE_LENGTH: usize = 64;
pub const MAX_CREDENTIALS_ARRAY_SIZE: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositPreauthCredential {
    pub issuer: AccountID,
    pub credential_type: Blob,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeFields {
    pub locking_chain_door: AccountID,
    pub locking_chain_issue: Issue,
    pub issuing_chain_door: AccountID,
    pub issuing_chain_issue: Issue,
}

pub fn render_ledger_entry_json(node: &STLedgerEntry) -> JsonValue {
    match node.get_type() {
        LedgerEntryType::Bridge => Bridge::new(Arc::new(node.clone()))
            .map(|bridge| bridge.as_st_ledger_entry().json(JsonOptions::NONE))
            .unwrap_or_else(|_| node.json(JsonOptions::NONE)),
        LedgerEntryType::XChainOwnedClaimId => XChainOwnedClaimID::new(Arc::new(node.clone()))
            .map(|entry| entry.as_st_ledger_entry().json(JsonOptions::NONE))
            .unwrap_or_else(|_| node.json(JsonOptions::NONE)),
        LedgerEntryType::XChainOwnedCreateAccountClaimId => {
            XChainOwnedCreateAccountClaimID::new(Arc::new(node.clone()))
                .map(|entry| entry.as_st_ledger_entry().json(JsonOptions::NONE))
                .unwrap_or_else(|_| node.json(JsonOptions::NONE))
        }
        _ => node.json(JsonOptions::NONE),
    }
}

fn make_error(code: RpcErrorCode, message: impl Into<String>) -> JsonValue {
    let mut json = JsonValue::Object(BTreeMap::new());
    RpcStatus::with_message(code, message).inject(&mut json);
    json
}

fn manual_invalid_params(error: &str, message: &str) -> JsonValue {
    JsonValue::Object(BTreeMap::from([
        ("error".to_owned(), JsonValue::String(error.to_owned())),
        ("error_code".to_owned(), JsonValue::Signed(31)),
        (
            "error_message".to_owned(),
            JsonValue::String(message.to_owned()),
        ),
    ]))
}

pub fn malformed_error(err: impl Into<String>, message: impl Into<String>) -> JsonValue {
    manual_invalid_params(&err.into(), &message.into())
}

pub fn missing_field_error(field: impl AsRef<str>) -> JsonValue {
    missing_field_error_with_code(field, "malformedRequest")
}

pub fn missing_field_error_with_code(field: impl AsRef<str>, err: impl AsRef<str>) -> JsonValue {
    manual_invalid_params(
        err.as_ref(),
        &RpcStatus::missing_field_message(field.as_ref()),
    )
}

pub fn expected_field_error(
    err: impl AsRef<str>,
    field: impl AsRef<str>,
    expected: impl AsRef<str>,
) -> JsonValue {
    manual_invalid_params(
        err.as_ref(),
        &RpcStatus::expected_field_message(field.as_ref(), expected.as_ref()),
    )
}

pub fn invalid_field_error(
    err: impl AsRef<str>,
    field: impl AsRef<str>,
    expected: impl AsRef<str>,
) -> JsonValue {
    expected_field_error(err, field, expected)
}

pub fn has_required(
    params: &JsonValue,
    fields: &[&str],
    err: Option<&str>,
) -> Result<(), JsonValue> {
    let JsonValue::Object(object) = params else {
        return Err(missing_field_error_with_code(
            fields.first().copied().unwrap_or(""),
            err.unwrap_or("malformedRequest"),
        ));
    };

    for field in fields {
        if !object.contains_key(*field) || matches!(object.get(*field), Some(JsonValue::Null)) {
            return Err(missing_field_error_with_code(
                field,
                err.unwrap_or("malformedRequest"),
            ));
        }
    }

    Ok(())
}

fn required_value<T>(
    params: &JsonValue,
    field: &'static str,
    err: &'static str,
    expected: &'static str,
    parse: impl Fn(&JsonValue) -> Option<T>,
) -> Result<T, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Err(missing_field_error_with_code(field, err));
    };

    let Some(value) = object.get(field) else {
        return Err(missing_field_error_with_code(field, err));
    };
    if matches!(value, JsonValue::Null) {
        return Err(missing_field_error_with_code(field, err));
    }

    parse(value).ok_or_else(|| invalid_field_error(err, field, expected))
}

pub fn parse_account_id(value: &JsonValue) -> Option<AccountID> {
    let JsonValue::String(text) = value else {
        return None;
    };

    let account = parse_base58_account_id(text)?;
    (!account.is_zero()).then_some(account)
}

pub fn required_account_id(
    params: &JsonValue,
    field: &'static str,
    err: &'static str,
) -> Result<AccountID, JsonValue> {
    required_value(params, field, err, "AccountID", parse_account_id)
}

pub fn parse_uint32(value: &JsonValue) -> Option<u32> {
    match value {
        JsonValue::Unsigned(value) => u32::try_from(*value).ok(),
        JsonValue::Signed(value) if *value >= 0 => u32::try_from(*value as u64).ok(),
        JsonValue::String(text) => text.parse::<u32>().ok(),
        _ => None,
    }
}

pub fn required_uint32(
    params: &JsonValue,
    field: &'static str,
    err: &'static str,
) -> Result<u32, JsonValue> {
    required_value(params, field, err, "number", parse_uint32)
}

pub fn parse_uint256(value: &JsonValue) -> Option<Uint256> {
    let JsonValue::String(text) = value else {
        return None;
    };

    Uint256::from_hex(text).ok()
}

pub fn required_uint256(
    params: &JsonValue,
    field: &'static str,
    err: &'static str,
) -> Result<Uint256, JsonValue> {
    required_value(params, field, err, "Hash256", parse_uint256)
}

pub fn parse_uint192(value: &JsonValue) -> Option<Uint192> {
    let JsonValue::String(text) = value else {
        return None;
    };

    Uint192::from_hex(text).ok()
}

pub fn required_uint192(
    params: &JsonValue,
    field: &'static str,
    err: &'static str,
) -> Result<Uint192, JsonValue> {
    required_value(params, field, err, "Hash192", parse_uint192)
}

pub fn parse_hex_blob(value: &JsonValue, max_length: usize) -> Option<Blob> {
    let JsonValue::String(text) = value else {
        return None;
    };

    let blob = str_unhex(text)?;
    (!blob.is_empty() && blob.len() <= max_length).then_some(blob)
}

pub fn required_hex_blob(
    params: &JsonValue,
    field: &'static str,
    max_length: usize,
    err: &'static str,
) -> Result<Blob, JsonValue> {
    required_value(params, field, err, "hex string", |value| {
        parse_hex_blob(value, max_length)
    })
}

pub fn parse_issue(value: &JsonValue) -> Option<Issue> {
    issue_from_json(value).ok()
}

pub fn required_issue(
    params: &JsonValue,
    field: &'static str,
    err: &'static str,
) -> Result<Issue, JsonValue> {
    required_value(params, field, err, "Issue", parse_issue)
}

pub fn parse_asset(value: &JsonValue) -> Option<Asset> {
    asset_from_json(value).ok()
}

pub fn required_asset(
    params: &JsonValue,
    field: &'static str,
    err: &'static str,
) -> Result<Asset, JsonValue> {
    required_value(params, field, err, "Asset", parse_asset)
}

pub fn parse_object_id(
    value: &JsonValue,
    field: &'static str,
    expected: &'static str,
) -> Result<Uint256, JsonValue> {
    parse_uint256(value).ok_or_else(|| invalid_field_error("malformedRequest", field, expected))
}

pub fn parse_fixed(
    keylet: Keylet,
    value: &JsonValue,
    field: &'static str,
) -> Result<Uint256, JsonValue> {
    if let JsonValue::Bool(false) = value {
        return Err(invalid_field_error("invalidParams", field, "true"));
    }
    if let JsonValue::Bool(true) = value {
        return Ok(keylet.key);
    }
    parse_object_id(value, field, "hex string")
}

pub fn parse_index(
    params: &JsonValue,
    field: &'static str,
    api_version: u32,
) -> Result<Uint256, JsonValue> {
    if api_version > 2 {
        if let JsonValue::String(index) = params {
            if index == "amendments" {
                return Ok(amendments_key());
            }
            if index == "fee" {
                return Ok(fee_settings_keylet().key);
            }
            if index == "nunl" {
                return Ok(negative_unl_keylet().key);
            }
            if index == "hashes" {
                return Ok(skip_keylet().key);
            }
        }
    }

    match params {
        JsonValue::Unsigned(value) => Ok(skip_keylet_for_ledger(*value as u32).key),
        JsonValue::Signed(value) if *value >= 0 => Ok(skip_keylet_for_ledger(*value as u32).key),
        _ => parse_object_id(params, field, "hex string"),
    }
}

pub fn parse_account_root(
    params: &JsonValue,
    field: &'static str,
    _api_version: u32,
) -> Result<Uint256, JsonValue> {
    required_value(params, field, "malformedAddress", "AccountID", |value| {
        parse_account_id(value)
            .map(|account| {
                account_keylet(Uint160::from_slice(account.data()).expect("account width"))
            })
            .map(|keylet| keylet.key)
    })
}

pub fn parse_directory_node(
    params: &JsonValue,
    field: &'static str,
    _api_version: u32,
) -> Result<Uint256, JsonValue> {
    let JsonValue::Object(object) = params else {
        return parse_object_id(params, field, "hex string");
    };

    if matches!(object.get("sub_index"), Some(JsonValue::Bool(_))) {
        return Err(invalid_field_error(
            "malformedRequest",
            "sub_index",
            "number",
        ));
    }

    let has_owner = object.contains_key("owner");
    let has_dir_root = object.contains_key("dir_root");
    if has_owner == has_dir_root {
        return Err(malformed_error(
            "malformedRequest",
            "Must have exactly one of `owner` and `dir_root` fields.",
        ));
    }

    let sub_index = match object.get("sub_index") {
        Some(JsonValue::Unsigned(value)) => *value,
        Some(JsonValue::Signed(value)) if *value >= 0 => *value as u64,
        Some(JsonValue::Null) | None => 0,
        _ => {
            return Err(invalid_field_error(
                "malformedRequest",
                "sub_index",
                "number",
            ));
        }
    };

    if let Some(dir_root) = object.get("dir_root") {
        let root = parse_uint256(dir_root)
            .ok_or_else(|| invalid_field_error("malformedDirRoot", "dir_root", "hash"))?;
        return Ok(page_keylet(Keylet::new(LedgerEntryType::DirectoryNode, root), sub_index).key);
    }

    let owner = required_account_id(params, "owner", "malformedAddress")?;
    Ok(page_keylet(
        owner_dir_keylet(Uint160::from_slice(owner.data()).expect("account width")),
        sub_index,
    )
    .key)
}

pub fn parse_amendments(
    params: &JsonValue,
    field: &'static str,
    _api_version: u32,
) -> Result<Uint256, JsonValue> {
    parse_fixed(
        Keylet::new(LedgerEntryType::Amendments, amendments_key()),
        params,
        field,
    )
}

pub fn parse_fee_settings(
    params: &JsonValue,
    field: &'static str,
    _api_version: u32,
) -> Result<Uint256, JsonValue> {
    parse_fixed(fee_settings_keylet(), params, field)
}

pub fn parse_negative_unl(
    params: &JsonValue,
    field: &'static str,
    _api_version: u32,
) -> Result<Uint256, JsonValue> {
    parse_fixed(negative_unl_keylet(), params, field)
}

pub fn parse_ledger_hashes(
    params: &JsonValue,
    field: &'static str,
    api_version: u32,
) -> Result<Uint256, JsonValue> {
    if matches!(params, JsonValue::Unsigned(_) | JsonValue::Signed(_)) {
        return parse_index(params, field, api_version);
    }
    parse_fixed(skip_keylet(), params, field)
}

pub fn parse_account_objects_directory(
    params: &JsonValue,
    field: &'static str,
) -> Result<Uint256, JsonValue> {
    let JsonValue::Object(object) = params else {
        return parse_object_id(params, field, "hex string");
    };

    if matches!(object.get("sub_index"), Some(JsonValue::Bool(_))) {
        return Err(invalid_field_error(
            "malformedRequest",
            "sub_index",
            "number",
        ));
    }

    let has_owner = object.contains_key("owner");
    let has_dir_root = object.contains_key("dir_root");
    if has_owner == has_dir_root {
        return Err(malformed_error(
            "malformedRequest",
            "Must have exactly one of `owner` and `dir_root` fields.",
        ));
    }

    let sub_index = match object.get("sub_index") {
        Some(JsonValue::Unsigned(value)) => *value,
        Some(JsonValue::Signed(value)) if *value >= 0 => *value as u64,
        Some(JsonValue::Null) | None => 0,
        _ => {
            return Err(invalid_field_error(
                "malformedRequest",
                "sub_index",
                "number",
            ));
        }
    };

    if let Some(dir_root) = object.get("dir_root") {
        let root = parse_uint256(dir_root)
            .ok_or_else(|| invalid_field_error("malformedDirRoot", "dir_root", "hash"))?;
        return Ok(page_keylet(Keylet::new(LedgerEntryType::DirectoryNode, root), sub_index).key);
    }

    let owner = parse_account_id(object.get("owner").expect("owner must be present"))
        .ok_or_else(|| invalid_field_error("malformedAddress", "owner", "AccountID"))?;
    Ok(page_keylet(
        owner_dir_keylet(Uint160::from_slice(owner.data()).expect("account width")),
        sub_index,
    )
    .key)
}

pub fn parse_escrow(params: &JsonValue, _field: &'static str) -> Result<Uint256, JsonValue> {
    let owner = required_account_id(params, "owner", "malformedOwner")?;
    let seq = required_uint32(params, "seq", "malformedSeq")?;
    Ok(escrow_keylet(
        Uint160::from_slice(owner.data()).expect("account width"),
        seq,
    )
    .key)
}

pub fn parse_loan_broker(params: &JsonValue, _field: &'static str) -> Result<Uint256, JsonValue> {
    let owner = required_account_id(params, "owner", "malformedOwner")?;
    let seq = required_uint32(params, "seq", "malformedSeq")?;
    Ok(loan_broker_keylet(
        Uint160::from_slice(owner.data()).expect("account width"),
        seq,
    )
    .key)
}

pub fn parse_loan(params: &JsonValue, _field: &'static str) -> Result<Uint256, JsonValue> {
    let id = required_uint256(params, "loan_broker_id", "malformedBroker")?;
    let seq = required_uint32(params, "loan_seq", "malformedSeq")?;
    Ok(loan_keylet(id, seq).key)
}

pub fn parse_mptoken(params: &JsonValue, field: &'static str) -> Result<Uint256, JsonValue> {
    if !matches!(params, JsonValue::Object(_)) {
        return parse_object_id(params, field, "hex string");
    }

    let issuance_id = required_uint192(params, "mpt_issuance_id", "malformedMPTIssuanceID")?;
    let account = required_account_id(params, "account", "malformedAccount")?;
    Ok(mptoken_keylet_from_mptid(
        issuance_id,
        Uint160::from_slice(account.data()).expect("account width"),
    )
    .key)
}

pub fn parse_mpt_issuance(params: &JsonValue, field: &'static str) -> Result<Uint256, JsonValue> {
    let Some(issuance_id) = parse_uint192(params) else {
        return Err(invalid_field_error(
            "malformedMPTokenIssuance",
            field,
            "Hash192",
        ));
    };

    Ok(mpt_issuance_keylet_from_mptid(issuance_id).key)
}

pub fn parse_offer(params: &JsonValue, _field: &'static str) -> Result<Uint256, JsonValue> {
    let account = required_account_id(params, "account", "malformedAddress")?;
    let seq = required_uint32(params, "seq", "malformedRequest")?;
    Ok(offer_keylet(
        Uint160::from_slice(account.data()).expect("account width"),
        seq,
    )
    .key)
}

pub fn parse_oracle(params: &JsonValue, _field: &'static str) -> Result<Uint256, JsonValue> {
    let account = required_account_id(params, "account", "malformedAccount")?;
    let document_id = required_uint32(params, "oracle_document_id", "malformedDocumentID")?;
    Ok(oracle_keylet(
        Uint160::from_slice(account.data()).expect("account width"),
        document_id,
    )
    .key)
}

pub fn parse_pay_channel(params: &JsonValue, field: &'static str) -> Result<Uint256, JsonValue> {
    parse_object_id(params, field, "hex string")
}

pub fn parse_permissioned_domain(
    params: &JsonValue,
    field: &'static str,
) -> Result<Uint256, JsonValue> {
    if let JsonValue::String(_) = params {
        return parse_object_id(params, field, "hex string");
    }

    if !matches!(params, JsonValue::Object(_)) {
        return Err(invalid_field_error(
            "malformedRequest",
            field,
            "hex string or object",
        ));
    }

    let account = required_account_id(params, "account", "malformedAddress")?;
    let seq = required_uint32(params, "seq", "malformedRequest")?;
    Ok(permissioned_domain_keylet(
        Uint160::from_slice(account.data()).expect("account width"),
        seq,
    )
    .key)
}

pub fn parse_ripple_state(params: &JsonValue, field: &'static str) -> Result<Uint256, JsonValue> {
    let JsonValue::Object(object) = params else {
        return parse_object_id(params, field, "hex string");
    };

    if !object.contains_key("currency") || !object.contains_key("accounts") {
        return Err(malformed_error(
            "malformedRequest",
            "Must have `currency` and `accounts` fields.",
        ));
    }

    let JsonValue::Array(accounts) = object.get("accounts").expect("accounts must exist") else {
        return Err(invalid_field_error(
            "malformedRequest",
            "accounts",
            "length-2 array of Accounts",
        ));
    };
    if accounts.len() != 2 {
        return Err(invalid_field_error(
            "malformedRequest",
            "accounts",
            "length-2 array of Accounts",
        ));
    }

    let left = parse_account_id(&accounts[0])
        .ok_or_else(|| invalid_field_error("malformedAddress", "accounts", "array of Accounts"))?;
    let right = parse_account_id(&accounts[1])
        .ok_or_else(|| invalid_field_error("malformedAddress", "accounts", "array of Accounts"))?;
    if left == right {
        return Err(malformed_error(
            "malformedRequest",
            "Cannot have a trustline to self.",
        ));
    }

    let JsonValue::String(currency_text) = object.get("currency").expect("currency must exist")
    else {
        return Err(invalid_field_error(
            "malformedCurrency",
            "currency",
            "Currency",
        ));
    };
    let mut currency = Currency::zero();
    if !to_currency(&mut currency, currency_text) {
        return Err(invalid_field_error(
            "malformedCurrency",
            "currency",
            "Currency",
        ));
    }

    Ok(line(left, right, currency).key)
}

pub fn parse_signer_list(params: &JsonValue, field: &'static str) -> Result<Uint256, JsonValue> {
    parse_object_id(params, field, "hex string")
}

pub fn parse_ticket(params: &JsonValue, _field: &'static str) -> Result<Uint256, JsonValue> {
    let account = required_account_id(params, "account", "malformedAddress")?;
    let ticket_seq = required_uint32(params, "ticket_seq", "malformedRequest")?;
    Ok(ticket_keylet(
        Uint160::from_slice(account.data()).expect("account width"),
        ticket_seq,
    )
    .key)
}

pub fn parse_vault(params: &JsonValue, _field: &'static str) -> Result<Uint256, JsonValue> {
    let owner = required_account_id(params, "owner", "malformedOwner")?;
    let seq = required_uint32(params, "seq", "malformedRequest")?;
    Ok(vault_keylet(
        Uint160::from_slice(owner.data()).expect("account width"),
        seq,
    )
    .key)
}

pub fn parse_check(params: &JsonValue, field: &'static str) -> Result<Uint256, JsonValue> {
    parse_object_id(params, field, "hex string")
}

pub fn parse_did(params: &JsonValue, field: &'static str) -> Result<Uint256, JsonValue> {
    let account = required_account_id(params, field, "malformedAddress")?;
    Ok(did_keylet(Uint160::from_slice(account.data()).expect("account width")).key)
}

pub fn parse_delegate(params: &JsonValue, _field: &'static str) -> Result<Uint256, JsonValue> {
    let account = required_account_id(params, "account", "malformedAddress")?;
    let authorize = required_account_id(params, "authorize", "malformedAddress")?;
    Ok(delegate_keylet(
        Uint160::from_slice(account.data()).expect("account width"),
        Uint160::from_slice(authorize.data()).expect("account width"),
    )
    .key)
}

pub fn parse_nftoken_offer(params: &JsonValue, field: &'static str) -> Result<Uint256, JsonValue> {
    parse_object_id(params, field, "hex string")
}

pub fn parse_nftoken_page(params: &JsonValue, field: &'static str) -> Result<Uint256, JsonValue> {
    parse_object_id(params, field, "hex string")
}

pub fn parse_credential(params: &JsonValue, field: &'static str) -> Result<Uint256, JsonValue> {
    let JsonValue::Object(_) = params else {
        return parse_object_id(params, field, "hex string");
    };

    let subject = required_account_id(params, "subject", "malformedRequest")?;
    let issuer = required_account_id(params, "issuer", "malformedRequest")?;
    let cred_type = required_hex_blob(
        params,
        "credential_type",
        MAX_CREDENTIAL_TYPE_LENGTH,
        "malformedRequest",
    )?;
    Ok(credential_keylet(
        Uint160::from_slice(subject.data()).expect("account width"),
        Uint160::from_slice(issuer.data()).expect("account width"),
        &cred_type,
    )
    .key)
}

pub fn parse_bridge(params: &JsonValue, field: &'static str) -> Result<Uint256, JsonValue> {
    let JsonValue::Object(object) = params else {
        return parse_object_id(params, field, "hex string or object");
    };

    let bridge_value = object
        .get("bridge")
        .ok_or_else(|| missing_field_error("bridge"))?;
    if let JsonValue::String(_) = bridge_value {
        return parse_object_id(bridge_value, field, "hex string or object");
    }

    let bridge = parse_bridge_fields(bridge_value)?;
    let bridge_account = required_account_id(params, "bridge_account", "malformedBridgeAccount")?;

    if bridge_account == bridge.locking_chain_door {
        return Ok(bridge_keylet_from_door_issue(
            Uint160::from_slice(bridge.locking_chain_door.data()).expect("account width"),
            bridge.locking_chain_issue,
        )
        .key);
    }
    if bridge_account == bridge.issuing_chain_door {
        return Ok(bridge_keylet_from_door_issue(
            Uint160::from_slice(bridge.issuing_chain_door.data()).expect("account width"),
            bridge.issuing_chain_issue,
        )
        .key);
    }

    Err(malformed_error("malformedRequest", ""))
}

pub fn parse_bridge_fields(params: &JsonValue) -> Result<BridgeFields, JsonValue> {
    has_required(
        params,
        &[
            "LockingChainDoor",
            "LockingChainIssue",
            "IssuingChainDoor",
            "IssuingChainIssue",
        ],
        Some("malformedRequest"),
    )?;

    let JsonValue::Object(_object) = params else {
        return Err(malformed_error(
            "malformedRequest",
            "Must be an object with bridge fields.",
        ));
    };

    let locking_chain_door =
        required_account_id(params, "LockingChainDoor", "malformedLockingChainDoor")?;
    let issuing_chain_door =
        required_account_id(params, "IssuingChainDoor", "malformedIssuingChainDoor")?;
    let locking_chain_issue = required_issue(params, "LockingChainIssue", "malformedIssue")?;
    let issuing_chain_issue = required_issue(params, "IssuingChainIssue", "malformedIssue")?;

    Ok(BridgeFields {
        locking_chain_door,
        locking_chain_issue,
        issuing_chain_door,
        issuing_chain_issue,
    })
}

pub fn parse_xchain_owned_claim_id(
    params: &JsonValue,
    field: &'static str,
) -> Result<Uint256, JsonValue> {
    let JsonValue::Object(_) = params else {
        return parse_object_id(params, field, "hex string");
    };

    let bridge = parse_bridge_fields(params)?;
    let seq = required_uint32(
        params,
        "xchain_owned_claim_id",
        "malformedXChainOwnedClaimID",
    )?;
    Ok(xchain_owned_claim_id_keylet_from_bridge(
        Uint160::from_slice(bridge.locking_chain_door.data()).expect("account width"),
        bridge.locking_chain_issue,
        Uint160::from_slice(bridge.issuing_chain_door.data()).expect("account width"),
        bridge.issuing_chain_issue,
        u64::from(seq),
    )
    .key)
}

pub fn parse_xchain_owned_create_account_claim_id(
    params: &JsonValue,
    field: &'static str,
) -> Result<Uint256, JsonValue> {
    let JsonValue::Object(_) = params else {
        return parse_object_id(params, field, "hex string");
    };

    let bridge = parse_bridge_fields(params)?;
    let seq = required_uint32(
        params,
        "xchain_owned_create_account_claim_id",
        "malformedXChainOwnedCreateAccountClaimID",
    )?;
    Ok(xchain_owned_create_account_claim_id_keylet_from_bridge(
        Uint160::from_slice(bridge.locking_chain_door.data()).expect("account width"),
        bridge.locking_chain_issue,
        Uint160::from_slice(bridge.issuing_chain_door.data()).expect("account width"),
        bridge.issuing_chain_issue,
        u64::from(seq),
    )
    .key)
}

pub fn parse_deposit_preauth_credential_array(
    params: &JsonValue,
    field: &'static str,
) -> Result<Vec<DepositPreauthCredential>, JsonValue> {
    let JsonValue::Array(credentials) = params else {
        return Err(invalid_field_error(
            "malformedAuthorizedCredentials",
            field,
            "array",
        ));
    };

    let size = credentials.len();
    if size == 0 {
        return Err(malformed_error(
            "malformedAuthorizedCredentials",
            format!("Invalid field '{field}', array empty."),
        ));
    }
    if size > MAX_CREDENTIALS_ARRAY_SIZE {
        return Err(malformed_error(
            "malformedAuthorizedCredentials",
            format!("Invalid field '{field}', array too long."),
        ));
    }

    let mut out = Vec::with_capacity(size);
    for cred in credentials {
        let JsonValue::Object(_) = cred else {
            return Err(invalid_field_error(
                "malformedAuthorizedCredentials",
                field,
                "array",
            ));
        };

        let issuer = required_account_id(cred, "issuer", "malformedAuthorizedCredentials")?;
        let credential_type = required_hex_blob(
            cred,
            "credential_type",
            MAX_CREDENTIAL_TYPE_LENGTH,
            "malformedAuthorizedCredentials",
        )?;
        if credential_type.is_empty() {
            return Err(invalid_field_error(
                "malformedAuthorizedCredentials",
                field,
                "array",
            ));
        }

        out.push(DepositPreauthCredential {
            issuer,
            credential_type,
        });
    }

    Ok(out)
}

fn sorted_authorized_credential_hashes(
    credentials: &[DepositPreauthCredential],
) -> Result<Vec<Uint256>, JsonValue> {
    let mut hashes = Vec::with_capacity(credentials.len());
    for credential in credentials {
        hashes.push(sha512_half_slices(&[
            credential.issuer.data(),
            credential.credential_type.as_slice(),
        ]));
    }
    hashes.sort();
    hashes.dedup();
    if hashes.len() != credentials.len() {
        return Err(invalid_field_error(
            "malformedAuthorizedCredentials",
            "authorized_credentials",
            "array",
        ));
    }
    Ok(hashes)
}

pub fn parse_deposit_preauth_account(
    params: &JsonValue,
    _field: &'static str,
) -> Result<Uint256, JsonValue> {
    let has_authorized = matches!(
        params,
        JsonValue::Object(object) if object.contains_key("authorized")
    );
    let has_authorized_credentials = matches!(
        params,
        JsonValue::Object(object) if object.contains_key("authorized_credentials")
    );

    if has_authorized == has_authorized_credentials {
        return Err(malformed_error(
            "malformedRequest",
            "Must have exactly one of `authorized` and `authorized_credentials`.",
        ));
    }

    let owner = required_account_id(params, "owner", "malformedOwner")?;

    if let JsonValue::Object(object) = params {
        if has_authorized {
            let authorized = parse_account_id(object.get("authorized").expect("authorized exists"))
                .ok_or_else(|| {
                    invalid_field_error("malformedAuthorized", "authorized", "AccountID")
                })?;
            return Ok(deposit_preauth_keylet(
                Uint160::from_slice(owner.data()).expect("account width"),
                Uint160::from_slice(authorized.data()).expect("account width"),
            )
            .key);
        }

        let _credentials = parse_deposit_preauth_credential_array(
            object
                .get("authorized_credentials")
                .expect("authorized_credentials exists"),
            "authorized_credentials",
        )?;
        let hashes = sorted_authorized_credential_hashes(&_credentials)?;
        return Ok(deposit_preauth_credentials_keylet(
            Uint160::from_slice(owner.data()).expect("account width"),
            &hashes,
        )
        .key);
    }

    Err(malformed_error(
        "malformedRequest",
        "Must have an object value.",
    ))
}

pub fn parse_account_objects_directory_root(owner: AccountID) -> Uint256 {
    owner_dir_keylet(Uint160::from_slice(owner.data()).expect("account width")).key
}

pub fn parse_amm(params: &JsonValue, field: &'static str) -> Result<Uint256, JsonValue> {
    let JsonValue::Object(object) = params else {
        return parse_object_id(params, field, "hex string");
    };

    if !object.contains_key("asset") || !object.contains_key("asset2") {
        return Err(malformed_error(
            "malformedRequest",
            "Must have `asset` and `asset2` fields.",
        ));
    }

    let asset = required_asset(params, "asset", "malformedRequest")?;
    let asset2 = required_asset(params, "asset2", "malformedRequest")?;
    Ok(protocol::amm(asset, asset2).key)
}
