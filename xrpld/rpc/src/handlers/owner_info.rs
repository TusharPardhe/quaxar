//! Read-only `owner_info` RPC slice.

use std::{collections::BTreeMap, sync::Arc};

use basics::base_uint::Uint256;
use protocol::{
    AccountID, JsonOptions, JsonValue, LedgerEntryType, Offer, RippleState, STLedgerEntry, StBase,
    get_field_by_symbol, parse_base58_account_id,
};

use crate::handlers::ledger_lookup::{LedgerLookupLedger, LedgerLookupSource};
use crate::status::RpcErrorCode;

pub trait OwnerInfoSource: LedgerLookupSource {
    fn read_owner_dir_page(
        &self,
        ledger: &LedgerLookupLedger,
        account_id: AccountID,
        page_index: u64,
    ) -> Option<STLedgerEntry>;

    fn read_child_entry(
        &self,
        ledger: &LedgerLookupLedger,
        entry_index: Uint256,
    ) -> Option<STLedgerEntry>;
}

fn json_value_as_string(value: &JsonValue) -> String {
    match value {
        JsonValue::String(text) => text.clone(),
        JsonValue::Signed(value) => value.to_string(),
        JsonValue::Unsigned(value) => value.to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => String::new(),
    }
}

fn make_error(code: RpcErrorCode) -> JsonValue {
    let mut json = JsonValue::Object(BTreeMap::new());
    crate::commands::rpc_helpers::inject_error(code, &mut json);
    json
}

fn parse_ident(params: &JsonValue) -> Result<String, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Err(crate::commands::rpc_helpers::missing_field_error("account"));
    };

    if let Some(account) = object.get("account") {
        return Ok(json_value_as_string(account));
    }

    if let Some(ident) = object.get("ident") {
        return Ok(json_value_as_string(ident));
    }

    Err(crate::commands::rpc_helpers::missing_field_error("account"))
}

fn collect_owner_info<S: OwnerInfoSource>(
    source: &S,
    ledger: &LedgerLookupLedger,
    account_id: AccountID,
) -> JsonValue {
    let mut result = BTreeMap::new();
    let mut page_index = 0u64;

    loop {
        let Some(page) = source.read_owner_dir_page(ledger, account_id, page_index) else {
            break;
        };

        for entry_index in page
            .get_field_v256(get_field_by_symbol("sfIndexes"))
            .value()
            .iter()
            .copied()
        {
            let Some(entry) = source.read_child_entry(ledger, entry_index) else {
                continue;
            };

            match entry.get_type() {
                LedgerEntryType::Offer => {
                    let offers = result
                        .entry("offers".to_owned())
                        .or_insert_with(|| JsonValue::Array(Vec::new()));
                    let JsonValue::Array(array) = offers else {
                        unreachable!("offers should stay an array");
                    };
                    let offer = Offer::new(Arc::new(entry))
                        .expect("owner info offer entries should wrap cleanly");
                    array.push(offer.as_st_ledger_entry().json(JsonOptions::NONE));
                }
                LedgerEntryType::RippleState => {
                    let ripple_lines = result
                        .entry("ripple_lines".to_owned())
                        .or_insert_with(|| JsonValue::Array(Vec::new()));
                    let JsonValue::Array(array) = ripple_lines else {
                        unreachable!("ripple_lines should stay an array");
                    };
                    let ripple_state = RippleState::new(Arc::new(entry))
                        .expect("owner info ripple state entries should wrap cleanly");
                    array.push(ripple_state.as_st_ledger_entry().json(JsonOptions::NONE));
                }
                LedgerEntryType::AccountRoot | LedgerEntryType::DirectoryNode => {}
                _ => {}
            }
        }

        let next = page.get_field_u64(get_field_by_symbol("sfIndexNext"));
        if next == 0 {
            break;
        }
        page_index = next;
    }

    JsonValue::Object(result)
}

pub fn do_owner_info<S: OwnerInfoSource>(params: &JsonValue, source: &S) -> JsonValue {
    let ident = match parse_ident(params) {
        Ok(ident) => ident,
        Err(error) => return error,
    };

    let Some(account_id) = parse_base58_account_id(&ident) else {
        return JsonValue::Object(BTreeMap::from([
            (
                "accepted".to_owned(),
                make_error(RpcErrorCode::ActMalformed),
            ),
            ("current".to_owned(), make_error(RpcErrorCode::ActMalformed)),
        ]));
    };

    let Some(closed_ledger) = source.get_closed_ledger() else {
        return make_error(RpcErrorCode::NotSynced);
    };
    let Some(current_ledger) = source.get_current_ledger() else {
        return make_error(RpcErrorCode::NotSynced);
    };

    JsonValue::Object(BTreeMap::from([
        (
            "accepted".to_owned(),
            collect_owner_info(source, &closed_ledger, account_id),
        ),
        (
            "current".to_owned(),
            collect_owner_info(source, &current_ledger, account_id),
        ),
    ]))
}
