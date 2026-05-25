//! Narrow `account_objects` RPC handler slice.
//!
//! This ports the the reference implementation account-owned object traversal shape onto the
//! existing Rust ledger and keylet helpers, without inventing an application
//! runtime seam. The shared traversal logic lives in
//! `account_objects_support.rs` so later account/nft handler work can reuse the
//! same read-only ledger walk rules.

#![allow(clippy::question_mark)]

use std::collections::BTreeMap;

use basics::base_uint::{Uint160, Uint256};
use protocol::{JsonValue, LedgerEntryType, account_keylet, parse_base58_account_id, to_base58};

use crate::commands::rpc_helpers::{
    choose_ledger_entry_type, inject_error, invalid_field_error, read_limit_field, rpc_error,
};
use crate::handlers::account_objects_support::{
    AccountObjectsMarker, AccountObjectsView, AccountTraversalError, account_objects_valid_type,
    collect_account_objects,
};
use crate::handlers::ledger_lookup::{
    LedgerLookupContext, LedgerLookupSource, RpcRole, lookup_ledger_with_result,
};
use crate::state::tuning::Tuning;
use crate::status::RpcErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountObjectsRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait AccountObjectsSource: LedgerLookupSource + AccountObjectsView {}

impl<T> AccountObjectsSource for T where T: LedgerLookupSource + AccountObjectsView {}

fn ensure_object(value: &mut JsonValue) -> &mut BTreeMap<String, JsonValue> {
    if !matches!(value, JsonValue::Object(_)) {
        *value = JsonValue::Object(BTreeMap::new());
    }

    let JsonValue::Object(object) = value else {
        unreachable!("json value should be an object");
    };
    object
}

fn account_string(params: &JsonValue) -> Result<String, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Err(crate::commands::rpc_helpers::missing_field_error("account"));
    };

    let Some(account) = object.get("account") else {
        return Err(crate::commands::rpc_helpers::missing_field_error("account"));
    };
    let JsonValue::String(account) = account else {
        return Err(invalid_field_error("account"));
    };

    Ok(account.clone())
}

fn parse_marker(params: &JsonValue) -> Result<(Uint256, Uint256), JsonValue> {
    let JsonValue::Object(object) = params else {
        return Ok((Uint256::zero(), Uint256::zero()));
    };

    let Some(marker) = object.get("marker") else {
        return Ok((Uint256::zero(), Uint256::zero()));
    };

    let JsonValue::String(marker) = marker else {
        return Err(crate::commands::rpc_helpers::expected_field_error(
            "marker", "string",
        ));
    };

    let Some((dir_index, entry_index)) = marker.split_once(',') else {
        return Err(invalid_field_error("marker"));
    };

    let Ok(dir_index) = Uint256::from_hex(dir_index) else {
        return Err(invalid_field_error("marker"));
    };
    let Ok(entry_index) = Uint256::from_hex(entry_index) else {
        return Err(invalid_field_error("marker"));
    };

    Ok((dir_index, entry_index))
}

fn format_marker(marker: &AccountObjectsMarker) -> String {
    match marker {
        AccountObjectsMarker::NftPage { page } => format!("0,{page}"),
        AccountObjectsMarker::Directory {
            dir_index,
            entry_index,
        } => format!("{dir_index},{entry_index}"),
    }
}

fn deletion_blocker_filter(params: &JsonValue) -> Option<Vec<LedgerEntryType>> {
    let JsonValue::Object(object) = params else {
        return None;
    };

    let Some(marker) = object.get("deletion_blockers_only") else {
        return None;
    };

    if !matches!(marker, JsonValue::Bool(true)) {
        return None;
    }

    const BLOCKERS: &[(&str, LedgerEntryType)] = &[
        ("check", LedgerEntryType::Check),
        ("escrow", LedgerEntryType::Escrow),
        ("nft_page", LedgerEntryType::NFTokenPage),
        ("payment_channel", LedgerEntryType::PayChannel),
        ("state", LedgerEntryType::RippleState),
        ("xchain_owned_claim_id", LedgerEntryType::XChainOwnedClaimId),
        (
            "xchain_owned_create_account_claim_id",
            LedgerEntryType::XChainOwnedCreateAccountClaimId,
        ),
        ("bridge", LedgerEntryType::Bridge),
        ("mpt_issuance", LedgerEntryType::MPTokenIssuance),
        ("mptoken", LedgerEntryType::MPToken),
        ("permissioned_domain", LedgerEntryType::PermissionedDomain),
        ("vault", LedgerEntryType::Vault),
    ];

    let selected = object.get("type");
    let mut filter = Vec::new();
    for (name, entry_type) in BLOCKERS {
        if let Some(JsonValue::String(ty)) = selected
            && ty != name
        {
            continue;
        }
        filter.push(*entry_type);
    }

    Some(filter)
}

fn parse_type_filter(params: &JsonValue) -> Result<Option<Vec<LedgerEntryType>>, JsonValue> {
    if let Some(filter) = deletion_blocker_filter(params) {
        return Ok(Some(filter));
    }

    let choice = match choose_ledger_entry_type(params) {
        Ok(choice) => choice,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return Err(error);
        }
    };

    let Some(entry_type) = choice else {
        return Ok(None);
    };

    if !account_objects_valid_type(entry_type) {
        return Err(invalid_field_error("type"));
    }

    if entry_type == LedgerEntryType::Any {
        return Ok(None);
    }

    Ok(Some(vec![entry_type]))
}

pub fn do_account_objects<S: AccountObjectsSource>(
    request: &AccountObjectsRequest<'_>,
    source: &S,
) -> JsonValue {
    tracing::trace!(target: "rpc", method = "account_objects", "account_objects query");
    let account_text = match account_string(request.params) {
        Ok(account) => account,
        Err(error) => return error,
    };

    let context = LedgerLookupContext {
        params: request.params,
        source,
        api_version: request.api_version,
        role: request.role,
    };

    let (ledger, mut result) = match lookup_ledger_with_result(&context) {
        Ok(result) => result,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
    };

    let Some(account_id) = parse_base58_account_id(&account_text) else {
        inject_error(RpcErrorCode::ActMalformed, &mut result);
        return result;
    };

    let account_index =
        account_keylet(Uint160::from_slice(account_id.data()).expect("account width"));
    let account_exists = match source.exists_entry(account_index) {
        Ok(exists) => exists,
        Err(_) => return rpc_error(RpcErrorCode::DbDeserialization),
    };
    if !account_exists {
        return rpc_error(RpcErrorCode::ActNotFound);
    }

    let type_filter = match parse_type_filter(request.params) {
        Ok(filter) => filter,
        Err(error) => {
            return error;
        }
    };

    let limit = match read_limit_field(request.params, request.role, Tuning::ACCOUNT_OBJECTS) {
        Ok(limit) => limit,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
    };

    let (dir_index, entry_index) = match parse_marker(request.params) {
        Ok(marker) => marker,
        Err(error) => return error,
    };

    let traversal = match collect_account_objects(
        source,
        account_id,
        type_filter.as_deref(),
        dir_index,
        entry_index,
        limit,
    ) {
        Ok(traversal) => traversal,
        Err(AccountTraversalError::InvalidMarker) => return invalid_field_error("marker"),
        Err(AccountTraversalError::Traversal(_)) => {
            return rpc_error(RpcErrorCode::DbDeserialization);
        }
    };

    let object = ensure_object(&mut result);
    object.insert(
        "account_objects".to_owned(),
        JsonValue::Array(traversal.items),
    );
    object.insert(
        "account".to_owned(),
        JsonValue::String(to_base58(account_id)),
    );

    if let Some(marker) = traversal.marker {
        object.insert("limit".to_owned(), JsonValue::Unsigned(u64::from(limit)));
        object.insert(
            "marker".to_owned(),
            JsonValue::String(format_marker(&marker)),
        );
    }

    // The lookup result is the shape the current RPC layer already returns for
    // ledger selection. We keep the traversal layer isolated from any runtime
    // load accounting until that seam is explicitly ported.
    let _ = ledger;

    result
}
