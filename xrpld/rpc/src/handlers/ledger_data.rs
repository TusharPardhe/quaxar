//! Ledger data RPC handler slice.

#![allow(clippy::unnecessary_lazy_evaluations)]

use basics::{base_uint::Uint256, str_hex::str_hex};
use protocol::{JsonValue, LedgerEntryType};
use std::collections::BTreeMap;

use crate::handlers::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcRole, RpcStatus, is_unlimited,
    lookup_ledger_with_result,
};
use crate::state::tuning::Tuning;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerDataEntry {
    pub key: Uint256,
    pub entry_type: LedgerEntryType,
    pub json: JsonValue,
    pub binary: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerDataResolved {
    pub base_json: JsonValue,
    pub ledger_json: JsonValue,
    pub entries: Vec<LedgerDataEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerDataRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait LedgerDataSource: LedgerLookupSource {
    fn resolve_ledger_data(
        &self,
        ledger: &LedgerLookupLedger,
        binary: bool,
    ) -> Result<LedgerDataResolved, RpcStatus>;
}

fn ensure_object(json: &mut JsonValue) -> &mut BTreeMap<String, JsonValue> {
    if !matches!(json, JsonValue::Object(_)) {
        *json = JsonValue::Object(BTreeMap::new());
    }

    let JsonValue::Object(object) = json else {
        unreachable!("json value should now be an object");
    };
    object
}

fn is_integral(value: &JsonValue) -> bool {
    matches!(value, JsonValue::Signed(_) | JsonValue::Unsigned(_))
}

fn as_limit(value: &JsonValue) -> Option<i64> {
    match value {
        JsonValue::Signed(value) => Some(*value),
        JsonValue::Unsigned(value) => i64::try_from(*value).ok(),
        _ => None,
    }
}

fn parse_binary(params: &JsonValue) -> bool {
    let JsonValue::Object(object) = params else {
        return false;
    };

    matches!(object.get("binary"), Some(JsonValue::Bool(true)))
}

fn parse_marker(params: &JsonValue) -> Result<Option<Uint256>, RpcStatus> {
    let JsonValue::Object(object) = params else {
        return Ok(None);
    };

    let Some(marker) = object.get("marker") else {
        return Ok(None);
    };

    let JsonValue::String(marker) = marker else {
        return Err(RpcStatus::expected_field_error("marker", "valid"));
    };

    Uint256::from_hex(marker)
        .map(Some)
        .map_err(|_| RpcStatus::expected_field_error("marker", "valid"))
}

fn parse_limit(params: &JsonValue) -> Result<Option<i64>, RpcStatus> {
    let JsonValue::Object(object) = params else {
        return Ok(None);
    };

    let Some(limit) = object.get("limit") else {
        return Ok(None);
    };

    if !is_integral(limit) {
        return Err(RpcStatus::expected_field_error("limit", "integer"));
    }

    as_limit(limit)
        .map(Some)
        .ok_or_else(|| RpcStatus::expected_field_error("limit", "integer"))
}

fn resolve_entry_type(filter: &str) -> Option<LedgerEntryType> {
    protocol::keylet::ledger_entry_type_catalog()
        .iter()
        .find_map(|item| {
            item.name
                .eq_ignore_ascii_case(filter)
                .then_some(item.entry_type)
        })
        .or_else(|| match filter {
            "nft_offer" => Some(LedgerEntryType::NFTokenOffer),
            "check" => Some(LedgerEntryType::Check),
            "did" => Some(LedgerEntryType::DID),
            "nunl" => Some(LedgerEntryType::NegativeUnl),
            "nft_page" => Some(LedgerEntryType::NFTokenPage),
            "signer_list" => Some(LedgerEntryType::SignerList),
            "ticket" => Some(LedgerEntryType::Ticket),
            "account" => Some(LedgerEntryType::AccountRoot),
            "directory" => Some(LedgerEntryType::DirectoryNode),
            "amendments" => Some(LedgerEntryType::Amendments),
            "hashes" => Some(LedgerEntryType::LedgerHashes),
            "bridge" => Some(LedgerEntryType::Bridge),
            "offer" => Some(LedgerEntryType::Offer),
            "deposit_preauth" => Some(LedgerEntryType::DepositPreauth),
            "xchain_owned_claim_id" => Some(LedgerEntryType::XChainOwnedClaimId),
            "state" => Some(LedgerEntryType::RippleState),
            "fee" => Some(LedgerEntryType::FeeSettings),
            "xchain_owned_create_account_claim_id" => {
                Some(LedgerEntryType::XChainOwnedCreateAccountClaimId)
            }
            "escrow" => Some(LedgerEntryType::Escrow),
            "payment_channel" => Some(LedgerEntryType::PayChannel),
            "amm" => Some(LedgerEntryType::AMM),
            "mpt_issuance" => Some(LedgerEntryType::MPTokenIssuance),
            "mptoken" => Some(LedgerEntryType::MPToken),
            "oracle" => Some(LedgerEntryType::Oracle),
            "credential" => Some(LedgerEntryType::Credential),
            "permissioned_domain" => Some(LedgerEntryType::PermissionedDomain),
            "delegate" => Some(LedgerEntryType::Delegate),
            "vault" => Some(LedgerEntryType::Vault),
            "loan_broker" => Some(LedgerEntryType::LoanBroker),
            "loan" => Some(LedgerEntryType::Loan),
            _ => None,
        })
}

pub fn choose_ledger_entry_type(params: &JsonValue) -> Result<LedgerEntryType, RpcStatus> {
    let JsonValue::Object(object) = params else {
        return Ok(LedgerEntryType::Any);
    };

    let Some(value) = object.get("type") else {
        return Ok(LedgerEntryType::Any);
    };

    let JsonValue::String(filter) = value else {
        return Err(RpcStatus::expected_field_error("type", "string"));
    };

    resolve_entry_type(filter).ok_or_else(|| RpcStatus::invalid_field_error("type"))
}

fn merge_object_fields(target: &mut BTreeMap<String, JsonValue>, source: JsonValue) {
    let JsonValue::Object(source) = source else {
        return;
    };

    for (key, value) in source {
        target.insert(key, value);
    }
}

pub fn do_ledger_data<S: LedgerDataSource>(
    request: &LedgerDataRequest<'_>,
    source: &S,
) -> JsonValue {
    tracing::trace!(target: "rpc", method = "ledger_data", "ledger_data query");
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

    let binary = parse_binary(request.params);
    let resolved = match source.resolve_ledger_data(&ledger, binary) {
        Ok(resolved) => resolved,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
    };

    let marker = match parse_marker(request.params) {
        Ok(marker) => marker,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
    };

    let mut limit = match parse_limit(request.params) {
        Ok(limit) => limit.unwrap_or(-1),
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
    };

    let max_limit = i64::from(Tuning::page_length(binary));
    if limit < 0 || (limit > max_limit && !is_unlimited(request.role)) {
        limit = max_limit;
    }

    let type_filter = match choose_ledger_entry_type(request.params) {
        Ok(type_filter) => type_filter,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
    };

    let object = ensure_object(&mut result);
    merge_object_fields(object, resolved.base_json);

    if marker.is_none() {
        object.insert("ledger".to_owned(), resolved.ledger_json);
    }

    let mut nodes = Vec::new();
    let mut entries = resolved.entries;
    entries.sort_by(|left, right| left.key.cmp(&right.key));

    let start_key = marker.unwrap_or_default();
    let mut remaining = limit;

    for entry in entries.into_iter().filter(|entry| entry.key > start_key) {
        if remaining <= 0 {
            let mut marker_key = entry.key;
            marker_key.decrement();
            object.insert(
                "marker".to_owned(),
                JsonValue::String(marker_key.to_string()),
            );
            break;
        }

        remaining -= 1;

        if type_filter != LedgerEntryType::Any && type_filter != entry.entry_type {
            continue;
        }

        if binary {
            let mut node = BTreeMap::new();
            node.insert("data".to_owned(), JsonValue::String(str_hex(entry.binary)));
            node.insert("index".to_owned(), JsonValue::String(entry.key.to_string()));
            nodes.push(JsonValue::Object(node));
        } else {
            let mut node = entry.json;
            let node_object = ensure_object(&mut node);
            node_object.insert("index".to_owned(), JsonValue::String(entry.key.to_string()));
            nodes.push(node);
        }
    }

    object.insert("state".to_owned(), JsonValue::Array(nodes));
    result
}
