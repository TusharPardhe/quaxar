//! Narrow `vault_info` RPC handler slice.
//!
//! This ports the the reference implementation request parsing and result shaping without
//! inventing an `Application` or `ReadView` clone. The handler owns the
//! ledger lookup and validation steps, then delegates ledger-entry reads to an
//! explicit source trait.

use std::{collections::BTreeMap, sync::Arc};

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    JsonOptions, JsonValue, MPTokenIssuance, STLedgerEntry, StBase, Vault, get_field_by_symbol,
    mpt_issuance_keylet_from_mptid, parse_base58_account_id, vault_keylet, vault_keylet_from_key,
};

use crate::handlers::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcRole, lookup_ledger_with_result,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultInfoRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait VaultInfoSource: LedgerLookupSource {
    fn read_ledger_entry(
        &self,
        ledger: &LedgerLookupLedger,
        entry_index: Uint256,
    ) -> Option<STLedgerEntry>;
}

fn ensure_object(value: &mut JsonValue) -> &mut BTreeMap<String, JsonValue> {
    if !matches!(value, JsonValue::Object(_)) {
        *value = JsonValue::Object(BTreeMap::new());
    }

    let JsonValue::Object(object) = value else {
        unreachable!("json value should be an object");
    };
    object
}

fn set_error(result: &mut JsonValue, error: &str) {
    let object = ensure_object(result);
    object.insert("error".to_owned(), JsonValue::String(error.to_owned()));
}

fn parse_vault_id(params: &JsonValue) -> Result<Uint256, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Err(JsonValue::String("malformedRequest".to_owned()));
    };

    let has_vault_id = object.contains_key("vault_id");
    let has_owner = object.contains_key("owner");
    let has_seq = object.contains_key("seq");

    if has_vault_id && !has_owner && !has_seq {
        let Some(JsonValue::String(text)) = object.get("vault_id") else {
            return Err(JsonValue::String("malformedRequest".to_owned()));
        };

        if text.len() != Uint256::BYTES * 2 {
            return Err(JsonValue::String("malformedRequest".to_owned()));
        }

        let key = Uint256::from_hex(text)
            .map_err(|_| JsonValue::String("malformedRequest".to_owned()))?;
        if key.is_zero() {
            return Err(JsonValue::String("malformedRequest".to_owned()));
        }
        return Ok(key);
    }

    if !has_vault_id && has_owner && has_seq {
        let Some(JsonValue::String(owner_text)) = object.get("owner") else {
            return Err(JsonValue::String("malformedRequest".to_owned()));
        };
        let Some(owner) = parse_base58_account_id(owner_text) else {
            return Err(JsonValue::String("malformedRequest".to_owned()));
        };

        let seq = match object.get("seq") {
            Some(JsonValue::Unsigned(value)) if *value > 0 && *value <= u64::from(u32::MAX) => {
                *value as u32
            }
            Some(JsonValue::Signed(value)) if *value > 0 && *value <= i64::from(u32::MAX) => {
                *value as u32
            }
            _ => return Err(JsonValue::String("malformedRequest".to_owned())),
        };

        return Ok(vault_keylet(
            Uint160::from_slice(owner.data()).expect("account width"),
            seq,
        )
        .key);
    }

    Err(JsonValue::String("malformedRequest".to_owned()))
}

fn read_vault_info<S: VaultInfoSource>(
    source: &S,
    ledger: &LedgerLookupLedger,
    vault_key: Uint256,
    result: &mut JsonValue,
) -> bool {
    let Some(sle_vault) = source.read_ledger_entry(ledger, vault_keylet_from_key(vault_key).key)
    else {
        set_error(result, "entryNotFound");
        return false;
    };

    let share_mpt_id = sle_vault.get_field_h192(get_field_by_symbol("sfShareMPTID"));
    let Some(sle_issuance) =
        source.read_ledger_entry(ledger, mpt_issuance_keylet_from_mptid(share_mpt_id).key)
    else {
        set_error(result, "entryNotFound");
        return false;
    };

    let vault_wrapper =
        Vault::new(Arc::new(sle_vault)).expect("vault_info should only read Vault entries here");
    let issuance_wrapper = MPTokenIssuance::new(Arc::new(sle_issuance))
        .expect("vault_info should only read MPTokenIssuance entries here");

    let mut vault = vault_wrapper.as_st_ledger_entry().json(JsonOptions::NONE);
    let JsonValue::Object(vault_object) = &mut vault else {
        unreachable!("STLedgerEntry::json must produce an object");
    };
    vault_object.insert(
        "shares".to_owned(),
        issuance_wrapper
            .as_st_ledger_entry()
            .json(JsonOptions::NONE),
    );

    let object = ensure_object(result);
    object.insert("vault".to_owned(), vault);
    true
}

pub fn do_vault_info<S: VaultInfoSource>(request: &VaultInfoRequest<'_>, source: &S) -> JsonValue {
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

    let vault_key = match parse_vault_id(request.params) {
        Ok(key) => key,
        Err(error) => {
            if let JsonValue::String(token) = error {
                set_error(&mut result, &token);
            }
            return result;
        }
    };

    if !read_vault_info(source, &ledger, vault_key, &mut result) {
        return result;
    }

    result
}
