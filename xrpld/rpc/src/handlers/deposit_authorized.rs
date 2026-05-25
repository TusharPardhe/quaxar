//! Read-only `deposit_authorized` RPC slice.
//!
//! This ports the the reference implementation handler shape onto explicit Rust ledger seams.
//! The handler keeps account parsing, credential validation, and response
//! shaping local without inventing an application/runtime owner.

use std::collections::{BTreeMap, BTreeSet};

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, JsonValue, STLedgerEntry, deposit_preauth_credentials_keylet,
    deposit_preauth_keylet, get_field_by_symbol, lsfAccepted, lsfDepositAuth,
    parse_base58_account_id, sha512_half_slices,
};

use crate::commands::rpc_helpers::{expected_field_error, missing_field_error, rpc_error};
use crate::handlers::ledger_entry_helpers::MAX_CREDENTIALS_ARRAY_SIZE;
use crate::handlers::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcRole, RpcStatus,
    lookup_ledger_with_result,
};
use crate::status::{RpcErrorCode, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepositAuthorizedRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait DepositAuthorizedSource: LedgerLookupSource {
    fn read_account_root(
        &self,
        ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Option<STLedgerEntry>;

    fn read_ledger_entry(
        &self,
        ledger: &LedgerLookupLedger,
        entry_index: Uint256,
    ) -> Option<STLedgerEntry>;

    fn parent_close_time(&self, ledger: &LedgerLookupLedger) -> u32;
}

fn json_to_error(status: RpcStatus) -> JsonValue {
    let mut json = JsonValue::Object(BTreeMap::new());
    status.inject(&mut json);
    json
}

fn bad_credentials(message: impl Into<String>) -> JsonValue {
    let mut json = JsonValue::Object(BTreeMap::new());
    Status::with_message(RpcErrorCode::BadCredentials, message).inject(&mut json);
    json
}

fn parse_required_account_text(
    params: &JsonValue,
    field: &'static str,
) -> Result<String, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Err(missing_field_error(field));
    };

    let Some(value) = object.get(field) else {
        return Err(missing_field_error(field));
    };
    let JsonValue::String(text) = value else {
        return Err(expected_field_error(field, "a string"));
    };

    Ok(text.clone())
}

fn parse_credentials(params: &JsonValue) -> Result<Option<Vec<String>>, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Ok(None);
    };

    let Some(credentials) = object.get("credentials") else {
        return Ok(None);
    };
    let JsonValue::Array(values) = credentials else {
        return Err(expected_field_error(
            "credentials",
            "is non-empty array of CredentialID(hash256)",
        ));
    };
    if values.is_empty() {
        return Err(expected_field_error(
            "credentials",
            "is non-empty array of CredentialID(hash256)",
        ));
    }
    if values.len() > MAX_CREDENTIALS_ARRAY_SIZE {
        return Err(expected_field_error("credentials", "array too long"));
    }

    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let JsonValue::String(text) = value else {
            return Err(expected_field_error(
                "credentials",
                "an array of CredentialID(hash256)",
            ));
        };
        out.push(text.clone());
    }

    Ok(Some(out))
}

fn parse_credential_key(text: &str) -> Result<Uint256, JsonValue> {
    Uint256::from_hex(text)
        .map_err(|_| expected_field_error("credentials", "an array of CredentialID(hash256)"))
}

fn validate_credentials<S: DepositAuthorizedSource>(
    source: &S,
    ledger: &LedgerLookupLedger,
    src_acct: AccountID,
    credentials: &[String],
) -> Result<BTreeSet<(AccountID, Vec<u8>)>, JsonValue> {
    let mut sorted = BTreeSet::<(AccountID, Vec<u8>)>::new();

    for cred in credentials {
        let cred_hash = parse_credential_key(cred)?;
        let Some(sle_cred) = source.read_ledger_entry(ledger, cred_hash) else {
            return Err(bad_credentials("credentials don't exist"));
        };

        if (sle_cred.get_flags() & lsfAccepted) == 0 {
            return Err(bad_credentials("credentials aren't accepted"));
        }

        let exp = if sle_cred.is_field_present(get_field_by_symbol("sfExpiration")) {
            sle_cred.get_field_u32(get_field_by_symbol("sfExpiration"))
        } else {
            u32::MAX
        };
        if source.parent_close_time(ledger) > exp {
            return Err(bad_credentials("credentials are expired"));
        }

        if sle_cred.get_account_id(get_field_by_symbol("sfSubject")) != src_acct {
            return Err(bad_credentials(
                "credentials doesn't belong to the root account",
            ));
        }

        let issuer = sle_cred.get_account_id(get_field_by_symbol("sfIssuer"));
        let credential_type = sle_cred.get_field_vl(get_field_by_symbol("sfCredentialType"));
        if !sorted.insert((issuer, credential_type)) {
            return Err(bad_credentials("duplicates in credentials"));
        }
    }

    Ok(sorted)
}

fn authorized_by_credentials<S: DepositAuthorizedSource>(
    source: &S,
    ledger: &LedgerLookupLedger,
    dst_acct: AccountID,
    sorted_credentials: &BTreeSet<(AccountID, Vec<u8>)>,
) -> bool {
    let hashes = sorted_credentials
        .iter()
        .map(|(issuer, credential_type)| {
            sha512_half_slices(&[issuer.data(), credential_type.as_slice()])
        })
        .collect::<Vec<_>>();
    let dst_owner = Uint160::from_slice(dst_acct.data()).expect("account width");
    source
        .read_ledger_entry(
            ledger,
            deposit_preauth_credentials_keylet(dst_owner, &hashes).key,
        )
        .is_some()
}

pub fn do_deposit_authorized<S: DepositAuthorizedSource>(
    request: &DepositAuthorizedRequest<'_>,
    source: &S,
) -> JsonValue {
    tracing::trace!(target: "rpc", method = "deposit_authorized", "deposit_authorized query");
    let src_text = match parse_required_account_text(request.params, "source_account") {
        Ok(text) => text,
        Err(error) => return error,
    };
    let dst_text = match parse_required_account_text(request.params, "destination_account") {
        Ok(text) => text,
        Err(error) => return error,
    };

    let Some(src_acct) = parse_base58_account_id(&src_text) else {
        return rpc_error(RpcErrorCode::ActMalformed);
    };
    let Some(dst_acct) = parse_base58_account_id(&dst_text) else {
        return rpc_error(RpcErrorCode::ActMalformed);
    };

    let context = LedgerLookupContext {
        params: request.params,
        source,
        api_version: request.api_version,
        role: request.role,
    };
    let (ledger, mut result) = match lookup_ledger_with_result(&context) {
        Ok(value) => value,
        Err(status) => return json_to_error(status),
    };

    if source.read_account_root(&ledger, src_acct).is_none() {
        Status::new(RpcErrorCode::SrcActNotFound).inject(&mut result);
        return result;
    }
    let Some(dst_root) = source.read_account_root(&ledger, dst_acct) else {
        Status::new(RpcErrorCode::DstActNotFound).inject(&mut result);
        return result;
    };

    let req_auth = (dst_root.get_flags() & lsfDepositAuth) != 0 && src_acct != dst_acct;

    let credentials_text = match parse_credentials(request.params) {
        Ok(credentials) => credentials,
        Err(error) => return error,
    };

    let validated_credentials = match credentials_text.as_ref() {
        Some(credentials) => match validate_credentials(source, &ledger, src_acct, credentials) {
            Ok(sorted) => Some(sorted),
            Err(error) => return error,
        },
        None => None,
    };

    let deposit_authorized = if req_auth {
        let dst_owner = Uint160::from_slice(dst_acct.data()).expect("account width");
        let src_owner = Uint160::from_slice(src_acct.data()).expect("account width");
        source
            .read_ledger_entry(&ledger, deposit_preauth_keylet(dst_owner, src_owner).key)
            .is_some()
            || validated_credentials
                .as_ref()
                .is_some_and(|sorted| authorized_by_credentials(source, &ledger, dst_acct, sorted))
    } else {
        true
    };

    let JsonValue::Object(object) = &mut result else {
        unreachable!("lookup_ledger_with_result always returns an object");
    };
    object.insert("source_account".to_owned(), JsonValue::String(src_text));
    object.insert(
        "destination_account".to_owned(),
        JsonValue::String(dst_text),
    );
    if let Some(credentials) = credentials_text {
        object.insert(
            "credentials".to_owned(),
            JsonValue::Array(credentials.into_iter().map(JsonValue::String).collect()),
        );
    }
    object.insert(
        "deposit_authorized".to_owned(),
        JsonValue::Bool(deposit_authorized),
    );

    result
}
