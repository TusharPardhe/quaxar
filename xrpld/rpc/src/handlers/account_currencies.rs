//! Read-only `account_currencies` RPC slice.

use std::collections::{BTreeMap, BTreeSet};

use basics::base_uint::Uint256;
use protocol::{
    AccountID, Currency, JsonValue, LedgerEntryType, STAmount, STLedgerEntry, bad_currency,
    currency_to_string, get_field_by_symbol, parse_base58_account_id,
};

use crate::commands::rpc_helpers::{invalid_field_error, missing_field_error, rpc_error};
use crate::handlers::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcRole, lookup_ledger_with_result,
};
use crate::status::RpcErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountCurrenciesRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait AccountCurrenciesSource: LedgerLookupSource {
    fn read_account_root(
        &self,
        ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Option<STLedgerEntry>;

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

#[derive(Debug, Clone)]
struct RpcTrustLine {
    low_limit: STAmount,
    high_limit: STAmount,
    balance: STAmount,
    view_lowest: bool,
}

impl RpcTrustLine {
    fn make_item(account_id: AccountID, sle: &STLedgerEntry) -> Option<Self> {
        if sle.get_type() != LedgerEntryType::RippleState {
            return None;
        }

        let low_limit = sle.get_field_amount(get_field_by_symbol("sfLowLimit"));
        let high_limit = sle.get_field_amount(get_field_by_symbol("sfHighLimit"));
        let view_lowest = low_limit.issue().account == account_id;
        let mut balance = sle.get_field_amount(get_field_by_symbol("sfBalance"));
        if !view_lowest {
            balance.negate();
        }

        Some(Self {
            low_limit,
            high_limit,
            balance,
            view_lowest,
        })
    }

    fn get_limit(&self) -> &STAmount {
        if self.view_lowest {
            &self.low_limit
        } else {
            &self.high_limit
        }
    }

    fn get_limit_peer(&self) -> &STAmount {
        if self.view_lowest {
            &self.high_limit
        } else {
            &self.low_limit
        }
    }

    fn get_balance(&self) -> &STAmount {
        &self.balance
    }
}

fn parse_account_text(params: &JsonValue) -> Result<String, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Err(missing_field_error("account"));
    };

    if let Some(account) = object.get("account") {
        let JsonValue::String(text) = account else {
            return Err(invalid_field_error("account"));
        };
        return Ok(text.clone());
    }

    if let Some(ident) = object.get("ident") {
        let JsonValue::String(text) = ident else {
            return Err(invalid_field_error("ident"));
        };
        return Ok(text.clone());
    }

    Err(missing_field_error("account"))
}

fn collect_trustline_currencies<S: AccountCurrenciesSource>(
    source: &S,
    ledger: &LedgerLookupLedger,
    account_id: AccountID,
) -> (BTreeSet<Currency>, BTreeSet<Currency>) {
    let mut send = BTreeSet::new();
    let mut receive = BTreeSet::new();
    let mut page_index = 0u64;

    loop {
        let Some(page) = source.read_owner_dir_page(ledger, account_id, page_index) else {
            break;
        };

        for entry_index in page
            .get_field_v256(get_field_by_symbol("sfIndexes"))
            .value()
        {
            let Some(sle) = source.read_child_entry(ledger, *entry_index) else {
                continue;
            };
            let Some(line) = RpcTrustLine::make_item(account_id, &sle) else {
                continue;
            };

            let balance = line.get_balance();
            if *balance < *line.get_limit() {
                receive.insert(balance.issue().currency);
            }

            let mut neg_balance = balance.clone();
            neg_balance.negate();
            if neg_balance < *line.get_limit_peer() {
                send.insert(balance.issue().currency);
            }
        }

        page_index = page.get_field_u64(get_field_by_symbol("sfIndexNext"));
        if page_index == 0 {
            break;
        }
    }

    send.remove(&bad_currency());
    receive.remove(&bad_currency());
    (send, receive)
}

fn currency_array(values: &BTreeSet<Currency>) -> JsonValue {
    JsonValue::Array(
        values
            .iter()
            .map(|currency| JsonValue::String(currency_to_string(*currency)))
            .collect(),
    )
}

pub fn do_account_currencies<S: AccountCurrenciesSource>(
    request: &AccountCurrenciesRequest<'_>,
    source: &S,
) -> JsonValue {
    tracing::trace!(target: "rpc", method = "account_currencies", "account_currencies query");
    let account_text = match parse_account_text(request.params) {
        Ok(text) => text,
        Err(error) => return error,
    };

    let Some(account_id) = parse_base58_account_id(&account_text) else {
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
        Err(status) => {
            let mut json = JsonValue::Object(BTreeMap::new());
            status.inject(&mut json);
            return json;
        }
    };

    if source.read_account_root(&ledger, account_id).is_none() {
        return rpc_error(RpcErrorCode::ActNotFound);
    }

    let (send, receive) = collect_trustline_currencies(source, &ledger, account_id);
    let JsonValue::Object(object) = &mut result else {
        return result;
    };
    object.insert("send_currencies".to_owned(), currency_array(&send));
    object.insert("receive_currencies".to_owned(), currency_array(&receive));
    result
}
