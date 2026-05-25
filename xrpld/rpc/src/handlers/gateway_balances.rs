//! Read-only `gateway_balances` RPC slice.
//!
//! This ports the the reference implementation gateway-balance scan onto the existing Rust
//! ledger seams without inventing Application/runtime ownership. The handler
//! keeps the account lookup, hotwallet validation, owner-directory traversal,
//! trustline shaping, escrow aggregation, and overflow handling explicit.

#![allow(
    clippy::too_many_arguments,
    clippy::unnecessary_cast,
    clippy::unwrap_or_default
)]

use std::collections::{BTreeMap, BTreeSet};

use basics::base_uint::Uint256;
use protocol::{
    AccountID, Currency, IOUAmount, JsonValue, LedgerEntryType, MAX_IOU_EXPONENT, MAX_IOU_MANTISSA,
    ST_AMOUNT_MAX_NATIVE_NETWORK, STAmount, STLedgerEntry, currency_to_string, get_field_by_symbol,
    lsfHighFreeze, lsfLowFreeze, parse_base58_account_id, to_base58, xrp_currency,
};

use crate::commands::rpc_helpers::{missing_field_error, rpc_error};
use crate::handlers::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcRole, lookup_ledger_with_result,
};
use crate::status::RpcErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayBalancesRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait GatewayBalancesSource: LedgerLookupSource {
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
struct GatewayTrustLine {
    balance: STAmount,
    flags: u32,
    view_lowest: bool,
}

impl GatewayTrustLine {
    fn make_item(account_id: AccountID, sle: &STLedgerEntry) -> Option<Self> {
        if sle.get_type() != LedgerEntryType::RippleState {
            return None;
        }

        let low_limit = sle.get_field_amount(get_field_by_symbol("sfLowLimit"));
        let view_lowest = low_limit.issue().account == account_id;
        let mut balance = sle.get_field_amount(get_field_by_symbol("sfBalance"));
        if !view_lowest {
            balance.negate();
        }

        Some(Self {
            balance,
            flags: sle.get_field_u32(get_field_by_symbol("sfFlags")),
            view_lowest,
        })
    }

    fn account_id_peer(&self, sle: &STLedgerEntry) -> AccountID {
        if self.view_lowest {
            sle.get_field_amount(get_field_by_symbol("sfHighLimit"))
                .issue()
                .account
        } else {
            sle.get_field_amount(get_field_by_symbol("sfLowLimit"))
                .issue()
                .account
        }
    }

    fn get_freeze(&self) -> bool {
        self.flags
            & if self.view_lowest {
                lsfLowFreeze
            } else {
                lsfHighFreeze
            }
            != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BalanceEntry {
    currency: Currency,
    amount: IOUAmount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LockedAmount {
    Native(i64),
    Iou(IOUAmount),
}

impl LockedAmount {
    fn from_amount(amount: &STAmount) -> Option<Self> {
        if amount.holds_mpt_issue() {
            return None;
        }

        if amount.native() {
            Some(Self::Native(amount.xrp().drops()))
        } else {
            Some(Self::Iou(amount.iou()))
        }
    }

    fn add_amount(&mut self, amount: &STAmount) {
        let Some(delta) = Self::from_amount(amount) else {
            return;
        };

        match (self, delta) {
            (Self::Native(total), Self::Native(delta)) => match total.checked_add(delta) {
                Some(sum) => *total = sum,
                None => *total = ST_AMOUNT_MAX_NATIVE_NETWORK as i64,
            },
            (Self::Iou(total), Self::Iou(delta)) => {
                if total.checked_add_assign(delta).is_err() {
                    *total = IOUAmount::from_parts(MAX_IOU_MANTISSA as i64, MAX_IOU_EXPONENT)
                        .expect("gateway balance max IOU amount should remain valid");
                }
            }
            _ => {}
        }
    }

    fn text(self) -> String {
        match self {
            Self::Native(drops) => drops.to_string(),
            Self::Iou(amount) => amount.to_string(),
        }
    }
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

fn json_value_as_string(value: &JsonValue) -> String {
    match value {
        JsonValue::String(text) => text.clone(),
        JsonValue::Signed(value) => value.to_string(),
        JsonValue::Unsigned(value) => value.to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => String::new(),
    }
}

fn parse_account(params: &JsonValue) -> Result<String, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Err(missing_field_error("account"));
    };

    if let Some(account) = object.get("account") {
        return Ok(json_value_as_string(account));
    }

    if let Some(ident) = object.get("ident") {
        return Ok(json_value_as_string(ident));
    }

    Err(missing_field_error("account"))
}

fn make_custom_error(token: &str, code: i64, message: &str) -> JsonValue {
    let mut object = BTreeMap::new();
    object.insert("error".to_owned(), JsonValue::String(token.to_owned()));
    object.insert("error_code".to_owned(), JsonValue::Signed(code));
    object.insert(
        "error_message".to_owned(),
        JsonValue::String(message.to_owned()),
    );
    JsonValue::Object(object)
}

fn invalid_hotwallet_error(api_version: u32) -> JsonValue {
    if api_version < 2 {
        make_custom_error("invalidHotWallet", 30, "Invalid hotwallet.")
    } else {
        rpc_error(RpcErrorCode::InvalidParams)
    }
}

fn parse_hotwallets(
    params: &JsonValue,
    api_version: u32,
) -> Result<BTreeSet<AccountID>, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Ok(BTreeSet::new());
    };

    let Some(hotwallet) = object.get("hotwallet") else {
        return Ok(BTreeSet::new());
    };

    let mut wallets = BTreeSet::new();
    let mut valid = true;

    let mut add_hotwallet = |value: &JsonValue| {
        if let JsonValue::String(text) = value
            && let Some(account_id) = parse_base58_account_id(text)
        {
            wallets.insert(account_id);
            return true;
        }

        false
    };

    match hotwallet {
        JsonValue::Null => {}
        JsonValue::String(_) => {
            valid &= add_hotwallet(hotwallet);
        }
        JsonValue::Array(values) => {
            for value in values {
                valid &= add_hotwallet(value);
            }
        }
        _ => {
            valid = false;
        }
    }

    if valid {
        Ok(wallets)
    } else {
        Err(invalid_hotwallet_error(api_version))
    }
}

fn add_amount_or_max(total: &mut IOUAmount, delta: IOUAmount) {
    if total.is_zero() {
        *total = delta;
        return;
    }

    if total.checked_add_assign(delta).is_err() {
        *total = IOUAmount::from_parts(MAX_IOU_MANTISSA as i64, MAX_IOU_EXPONENT)
            .expect("gateway balance max amount should remain valid");
    }
}

fn push_balance(
    balances: &mut BTreeMap<AccountID, Vec<BalanceEntry>>,
    account: AccountID,
    currency: Currency,
    amount: IOUAmount,
) {
    balances
        .entry(account)
        .or_default()
        .push(BalanceEntry { currency, amount });
}

fn format_balance_entries(entries: &[BalanceEntry]) -> JsonValue {
    JsonValue::Array(
        entries
            .iter()
            .map(|entry| {
                let mut object = BTreeMap::new();
                object.insert(
                    "currency".to_owned(),
                    JsonValue::String(currency_to_string(entry.currency)),
                );
                object.insert(
                    "value".to_owned(),
                    JsonValue::String(entry.amount.to_string()),
                );
                JsonValue::Object(object)
            })
            .collect(),
    )
}

fn populate_balance_map(
    result: &mut BTreeMap<String, JsonValue>,
    name: &str,
    balances: &BTreeMap<AccountID, Vec<BalanceEntry>>,
) {
    if balances.is_empty() {
        return;
    }

    let mut object = BTreeMap::new();
    for (account_id, entries) in balances {
        object.insert(to_base58(*account_id), format_balance_entries(entries));
    }
    result.insert(name.to_owned(), JsonValue::Object(object));
}

fn populate_currency_map(
    result: &mut BTreeMap<String, JsonValue>,
    name: &str,
    values: &BTreeMap<Currency, IOUAmount>,
) {
    if values.is_empty() {
        return;
    }

    let mut object = BTreeMap::new();
    for (currency, amount) in values {
        object.insert(
            currency_to_string(*currency),
            JsonValue::String(amount.to_string()),
        );
    }
    result.insert(name.to_owned(), JsonValue::Object(object));
}

fn populate_locked_map(
    result: &mut BTreeMap<String, JsonValue>,
    locked: &BTreeMap<Currency, LockedAmount>,
) {
    if locked.is_empty() {
        return;
    }

    let mut object = BTreeMap::new();
    for (currency, amount) in locked {
        object.insert(
            currency_to_string(*currency),
            JsonValue::String(amount.text()),
        );
    }
    result.insert("locked".to_owned(), JsonValue::Object(object));
}

fn traverse_owner_directory<S: GatewayBalancesSource>(
    source: &S,
    ledger: &LedgerLookupLedger,
    account_id: AccountID,
    hotwallets: &BTreeSet<AccountID>,
    obligations: &mut BTreeMap<Currency, IOUAmount>,
    balances: &mut BTreeMap<AccountID, Vec<BalanceEntry>>,
    frozen_balances: &mut BTreeMap<AccountID, Vec<BalanceEntry>>,
    assets: &mut BTreeMap<AccountID, Vec<BalanceEntry>>,
    locked: &mut BTreeMap<Currency, LockedAmount>,
) -> Result<(), JsonValue> {
    let mut page_index = 0u64;
    loop {
        let Some(owner_dir) = source.read_owner_dir_page(ledger, account_id, page_index) else {
            return Ok(());
        };

        for entry_index in owner_dir
            .get_field_v256(get_field_by_symbol("sfIndexes"))
            .value()
            .iter()
            .copied()
        {
            let Some(sle) = source.read_child_entry(ledger, entry_index) else {
                continue;
            };

            if sle.get_type() == LedgerEntryType::Escrow {
                let amount = sle.get_field_amount(get_field_by_symbol("sfAmount"));
                if !amount.holds_mpt_issue() {
                    let currency = if amount.native() {
                        xrp_currency()
                    } else {
                        amount.issue().currency
                    };
                    if let Some(total) = locked.get_mut(&currency) {
                        total.add_amount(&amount);
                    } else if let Some(total) = LockedAmount::from_amount(&amount) {
                        locked.insert(currency, total);
                    }
                }
            }

            let Some(trustline) = GatewayTrustLine::make_item(account_id, &sle) else {
                continue;
            };

            let peer = trustline.account_id_peer(&sle);
            let balance = trustline.balance.clone();
            let bal_sign = balance.signum();
            if bal_sign == 0 {
                continue;
            }

            let currency = balance.issue().currency;

            if hotwallets.contains(&peer) {
                push_balance(balances, peer, currency, -balance.iou());
            } else if bal_sign > 0 {
                push_balance(assets, peer, currency, balance.iou());
            } else if trustline.get_freeze() {
                push_balance(frozen_balances, peer, currency, -balance.iou());
            } else {
                let entry = obligations.entry(currency).or_insert_with(IOUAmount::new);
                add_amount_or_max(entry, -balance.iou());
            }
        }

        let next = owner_dir.get_field_u64(get_field_by_symbol("sfIndexNext"));
        if next == 0 {
            return Ok(());
        }
        page_index = next;
    }
}

pub fn do_gateway_balances<S: GatewayBalancesSource>(
    request: &GatewayBalancesRequest<'_>,
    source: &S,
) -> JsonValue {
    tracing::trace!(target: "rpc", method = "gateway_balances", "gateway_balances query");
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

    let account = match parse_account(request.params) {
        Ok(account) => account,
        Err(error) => return error,
    };

    let Some(account_id) = parse_base58_account_id(&account) else {
        let object = ensure_object(&mut result);
        object.insert("account".to_owned(), JsonValue::String(account));
        crate::commands::rpc_helpers::inject_error(RpcErrorCode::ActMalformed, &mut result);
        return result;
    };

    let object = ensure_object(&mut result);
    object.insert(
        "account".to_owned(),
        JsonValue::String(to_base58(account_id)),
    );

    if request.api_version > 1 && source.read_account_root(&ledger, account_id).is_none() {
        crate::commands::rpc_helpers::inject_error(RpcErrorCode::ActNotFound, &mut result);
        return result;
    }

    let hotwallets = match parse_hotwallets(request.params, request.api_version) {
        Ok(hotwallets) => hotwallets,
        Err(error) => return error,
    };

    let mut obligations = BTreeMap::<Currency, IOUAmount>::new();
    let mut balances = BTreeMap::<AccountID, Vec<BalanceEntry>>::new();
    let mut frozen_balances = BTreeMap::<AccountID, Vec<BalanceEntry>>::new();
    let mut assets = BTreeMap::<AccountID, Vec<BalanceEntry>>::new();
    let mut locked = BTreeMap::<Currency, LockedAmount>::new();

    if let Err(error) = traverse_owner_directory(
        source,
        &ledger,
        account_id,
        &hotwallets,
        &mut obligations,
        &mut balances,
        &mut frozen_balances,
        &mut assets,
        &mut locked,
    ) {
        return error;
    }

    let object = ensure_object(&mut result);
    populate_currency_map(object, "obligations", &obligations);
    populate_balance_map(object, "balances", &balances);
    populate_balance_map(object, "frozen_balances", &frozen_balances);
    populate_balance_map(object, "assets", &assets);
    populate_locked_map(object, &locked);

    result
}
