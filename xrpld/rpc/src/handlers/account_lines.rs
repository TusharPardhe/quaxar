//! Read-only `account_lines` RPC slice.

use std::collections::BTreeMap;

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, JsonValue, LedgerEntryType, STAmount, STLedgerEntry, StBase, currency_to_string,
    get_field_by_symbol, lsfHighAuth, lsfHighDeepFreeze, lsfHighFreeze, lsfHighNoRipple,
    lsfHighReserve, lsfLowAuth, lsfLowDeepFreeze, lsfLowFreeze, lsfLowNoRipple, lsfLowReserve,
    parse_base58_account_id, signers_keylet, to_base58,
};

use crate::commands::rpc_helpers::{expected_field_error, read_limit_field, rpc_error};
use crate::handlers::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcRole, lookup_ledger_with_result,
};
use crate::state::tuning::Tuning;
use crate::status::RpcErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountLinesRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait AccountLinesSource: LedgerLookupSource {
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
struct RPCTrustLine {
    low_limit: STAmount,
    high_limit: STAmount,
    balance: STAmount,
    flags: u32,
    view_lowest: bool,
    low_quality_in: u32,
    low_quality_out: u32,
    high_quality_in: u32,
    high_quality_out: u32,
}

impl RPCTrustLine {
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
            flags: sle.get_field_u32(get_field_by_symbol("sfFlags")),
            view_lowest,
            low_quality_in: sle.get_field_u32(get_field_by_symbol("sfLowQualityIn")),
            low_quality_out: sle.get_field_u32(get_field_by_symbol("sfLowQualityOut")),
            high_quality_in: sle.get_field_u32(get_field_by_symbol("sfHighQualityIn")),
            high_quality_out: sle.get_field_u32(get_field_by_symbol("sfHighQualityOut")),
        })
    }

    fn account_id_peer(&self) -> AccountID {
        if self.view_lowest {
            self.high_limit.issue().account
        } else {
            self.low_limit.issue().account
        }
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

    fn get_quality_in(&self) -> u32 {
        if self.view_lowest {
            self.low_quality_in
        } else {
            self.high_quality_in
        }
    }

    fn get_quality_out(&self) -> u32 {
        if self.view_lowest {
            self.low_quality_out
        } else {
            self.high_quality_out
        }
    }

    fn get_auth(&self) -> bool {
        self.flags
            & if self.view_lowest {
                lsfLowAuth
            } else {
                lsfHighAuth
            }
            != 0
    }

    fn get_auth_peer(&self) -> bool {
        self.flags
            & if self.view_lowest {
                lsfHighAuth
            } else {
                lsfLowAuth
            }
            != 0
    }

    fn get_no_ripple(&self) -> bool {
        self.flags
            & if self.view_lowest {
                lsfLowNoRipple
            } else {
                lsfHighNoRipple
            }
            != 0
    }

    fn get_no_ripple_peer(&self) -> bool {
        self.flags
            & if self.view_lowest {
                lsfHighNoRipple
            } else {
                lsfLowNoRipple
            }
            != 0
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

    fn get_freeze_peer(&self) -> bool {
        self.flags
            & if self.view_lowest {
                lsfHighFreeze
            } else {
                lsfLowFreeze
            }
            != 0
    }

    fn get_deep_freeze(&self) -> bool {
        self.flags
            & if self.view_lowest {
                lsfLowDeepFreeze
            } else {
                lsfHighDeepFreeze
            }
            != 0
    }

    fn get_deep_freeze_peer(&self) -> bool {
        self.flags
            & if self.view_lowest {
                lsfHighDeepFreeze
            } else {
                lsfLowDeepFreeze
            }
            != 0
    }
}

fn make_error(code: RpcErrorCode) -> JsonValue {
    let mut json = JsonValue::Object(BTreeMap::new());
    crate::commands::rpc_helpers::inject_error(code, &mut json);
    json
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
        return Err(crate::commands::rpc_helpers::missing_field_error("account"));
    };

    let Some(account) = object.get("account") else {
        return Err(crate::commands::rpc_helpers::missing_field_error("account"));
    };

    let JsonValue::String(account) = account else {
        return Err(crate::commands::rpc_helpers::invalid_field_error("account"));
    };

    Ok(account.clone())
}

fn parse_peer(params: &JsonValue) -> Option<String> {
    let JsonValue::Object(object) = params else {
        return None;
    };

    object
        .get("peer")
        .map(json_value_as_string)
        .filter(|peer| !peer.is_empty())
}

fn get_start_hint(sle: &STLedgerEntry, account_id: AccountID) -> u64 {
    if sle.get_type() == LedgerEntryType::RippleState {
        if sle
            .get_field_amount(get_field_by_symbol("sfLowLimit"))
            .issue()
            .account
            == account_id
        {
            return sle.get_field_u64(get_field_by_symbol("sfLowNode"));
        }
        if sle
            .get_field_amount(get_field_by_symbol("sfHighLimit"))
            .issue()
            .account
            == account_id
        {
            return sle.get_field_u64(get_field_by_symbol("sfHighNode"));
        }
    }

    if !sle.is_field_present(get_field_by_symbol("sfOwnerNode")) {
        return 0;
    }

    sle.get_field_u64(get_field_by_symbol("sfOwnerNode"))
}

fn is_related_to_account(sle: &STLedgerEntry, account_id: AccountID) -> bool {
    if sle.get_type() == LedgerEntryType::RippleState {
        return sle
            .get_field_amount(get_field_by_symbol("sfLowLimit"))
            .issue()
            .account
            == account_id
            || sle
                .get_field_amount(get_field_by_symbol("sfHighLimit"))
                .issue()
                .account
                == account_id;
    }

    if sle.is_field_present(get_field_by_symbol("sfAccount")) {
        return sle.get_account_id(get_field_by_symbol("sfAccount")) == account_id
            || (sle.is_field_present(get_field_by_symbol("sfDestination"))
                && sle.get_account_id(get_field_by_symbol("sfDestination")) == account_id);
    }

    if sle.get_type() == LedgerEntryType::SignerList {
        return *sle.key()
            == signers_keylet(Uint160::from_slice(account_id.data()).expect("account width")).key;
    }

    if sle.get_type() == LedgerEntryType::NFTokenOffer {
        return sle.get_account_id(get_field_by_symbol("sfOwner")) == account_id;
    }

    false
}

fn directory_contains_after<S: AccountLinesSource>(
    source: &S,
    ledger: &LedgerLookupLedger,
    account_id: AccountID,
    page_index: u64,
    after: Uint256,
) -> bool {
    let Some(page) = source.read_owner_dir_page(ledger, account_id, page_index) else {
        return false;
    };

    page.get_field_v256(get_field_by_symbol("sfIndexes"))
        .value()
        .contains(&after)
}

fn for_each_item_after<S, F>(
    source: &S,
    ledger: &LedgerLookupLedger,
    account_id: AccountID,
    after: Uint256,
    hint: u64,
    mut limit: u32,
    mut f: F,
) -> bool
where
    S: AccountLinesSource,
    F: FnMut(&STLedgerEntry) -> bool,
{
    let mut current_index = 0u64;

    if !after.is_zero() {
        if directory_contains_after(source, ledger, account_id, hint, after) {
            current_index = hint;
        }

        let mut found = false;
        loop {
            let Some(owner_dir) = source.read_owner_dir_page(ledger, account_id, current_index)
            else {
                return found;
            };

            for entry in owner_dir
                .get_field_v256(get_field_by_symbol("sfIndexes"))
                .value()
                .iter()
                .copied()
            {
                if !found {
                    if entry == after {
                        found = true;
                    }
                    continue;
                }

                let Some(sle) = source.read_child_entry(ledger, entry) else {
                    return false;
                };

                let keep = f(&sle);
                if keep && limit <= 1 {
                    return true;
                }
                if keep {
                    limit -= 1;
                }
            }

            let next = owner_dir.get_field_u64(get_field_by_symbol("sfIndexNext"));
            if next == 0 {
                return found;
            }
            current_index = next;
        }
    }

    loop {
        let Some(owner_dir) = source.read_owner_dir_page(ledger, account_id, current_index) else {
            return true;
        };

        for entry in owner_dir
            .get_field_v256(get_field_by_symbol("sfIndexes"))
            .value()
            .iter()
            .copied()
        {
            let Some(sle) = source.read_child_entry(ledger, entry) else {
                return false;
            };

            let keep = f(&sle);
            if keep && limit <= 1 {
                return true;
            }
            if keep {
                limit -= 1;
            }
        }

        let next = owner_dir.get_field_u64(get_field_by_symbol("sfIndexNext"));
        if next == 0 {
            return true;
        }
        current_index = next;
    }
}

fn ignore_default_line(line: &RPCTrustLine) -> bool {
    if line.view_lowest {
        line.flags & lsfLowReserve == 0
    } else {
        line.flags & lsfHighReserve == 0
    }
}

fn append_line_json(line: &RPCTrustLine) -> JsonValue {
    let mut object = BTreeMap::new();
    object.insert(
        "account".to_owned(),
        JsonValue::String(to_base58(line.account_id_peer())),
    );
    object.insert("balance".to_owned(), JsonValue::String(line.balance.text()));
    object.insert(
        "currency".to_owned(),
        JsonValue::String(currency_to_string(line.balance.issue().currency)),
    );
    object.insert(
        "limit".to_owned(),
        JsonValue::String(line.get_limit().text()),
    );
    object.insert(
        "limit_peer".to_owned(),
        JsonValue::String(line.get_limit_peer().text()),
    );
    object.insert(
        "quality_in".to_owned(),
        JsonValue::Unsigned(u64::from(line.get_quality_in())),
    );
    object.insert(
        "quality_out".to_owned(),
        JsonValue::Unsigned(u64::from(line.get_quality_out())),
    );

    if line.get_auth() {
        object.insert("authorized".to_owned(), JsonValue::Bool(true));
    }
    if line.get_auth_peer() {
        object.insert("peer_authorized".to_owned(), JsonValue::Bool(true));
    }
    if line.get_no_ripple() {
        object.insert("no_ripple".to_owned(), JsonValue::Bool(true));
    }
    if line.get_no_ripple_peer() {
        object.insert("no_ripple_peer".to_owned(), JsonValue::Bool(true));
    }
    if line.get_freeze() {
        object.insert("freeze".to_owned(), JsonValue::Bool(true));
    }
    if line.get_freeze_peer() {
        object.insert("freeze_peer".to_owned(), JsonValue::Bool(true));
    }
    if line.get_deep_freeze() {
        object.insert("deep_freeze".to_owned(), JsonValue::Bool(true));
    }
    if line.get_deep_freeze_peer() {
        object.insert("deep_freeze_peer".to_owned(), JsonValue::Bool(true));
    }

    JsonValue::Object(object)
}

pub fn do_account_lines<S: AccountLinesSource>(
    request: &AccountLinesRequest<'_>,
    source: &S,
) -> JsonValue {
    tracing::trace!(target: "rpc", method = "account_lines", "account_lines query");
    let account_text = match parse_account(request.params) {
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
        crate::commands::rpc_helpers::inject_error(RpcErrorCode::ActMalformed, &mut result);
        return result;
    };

    if source.read_account_root(&ledger, account_id).is_none() {
        return make_error(RpcErrorCode::ActNotFound);
    }

    let peer = if let Some(peer) = parse_peer(request.params) {
        let Some(peer_account) = parse_base58_account_id(&peer) else {
            crate::commands::rpc_helpers::inject_error(RpcErrorCode::ActMalformed, &mut result);
            return result;
        };
        Some(peer_account)
    } else {
        None
    };

    let ignore_default = matches!(
        request.params,
        JsonValue::Object(object) if matches!(object.get("ignore_default"), Some(JsonValue::Bool(true)))
    );

    let limit = match read_limit_field(request.params, request.role, Tuning::ACCOUNT_LINES) {
        Ok(limit) => limit,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
    };

    let JsonValue::Object(result_object) = &mut result else {
        unreachable!("result should be an object");
    };
    result_object.insert("lines".to_owned(), JsonValue::Array(Vec::new()));

    let mut start_after = Uint256::zero();
    let mut start_hint = 0u64;
    let marker_set =
        matches!(request.params, JsonValue::Object(object) if object.contains_key("marker"));
    if marker_set {
        let JsonValue::Object(object) = request.params else {
            unreachable!("marker parsing requires an object");
        };

        let Some(marker_value) = object.get("marker") else {
            return rpc_error(RpcErrorCode::InvalidParams);
        };

        let JsonValue::String(marker_text) = marker_value else {
            return expected_field_error("marker", "string");
        };

        let mut parts = marker_text.splitn(3, ',');
        let Some(first) = parts.next() else {
            return rpc_error(RpcErrorCode::InvalidParams);
        };
        if !start_after.parse_hex(first) {
            return rpc_error(RpcErrorCode::InvalidParams);
        }

        let Some(second) = parts.next() else {
            return rpc_error(RpcErrorCode::InvalidParams);
        };
        match second.parse::<u64>() {
            Ok(hint) => start_hint = hint,
            Err(_) => return rpc_error(RpcErrorCode::InvalidParams),
        }

        let Some(marker_sle) = source.read_child_entry(&ledger, start_after) else {
            return rpc_error(RpcErrorCode::InvalidParams);
        };
        if !is_related_to_account(&marker_sle, account_id) {
            return rpc_error(RpcErrorCode::InvalidParams);
        }
    }

    let mut count = 0u32;
    let mut marker: Option<Uint256> = None;
    let mut next_hint = 0u64;
    let mut items = Vec::new();

    if !for_each_item_after(
        source,
        &ledger,
        account_id,
        start_after,
        start_hint,
        limit + 1,
        |sle| {
            count = count.saturating_add(1);

            if count == limit {
                marker = Some(*sle.key());
                next_hint = get_start_hint(sle, account_id);
            }

            if sle.get_type() != LedgerEntryType::RippleState {
                return true;
            }

            let Some(line) = RPCTrustLine::make_item(account_id, sle) else {
                return true;
            };

            if ignore_default && ignore_default_line(&line) {
                return true;
            }

            if count <= limit
                && peer
                    .as_ref()
                    .is_none_or(|peer_account| *peer_account == line.account_id_peer())
            {
                items.push(append_line_json(&line));
            }

            true
        },
    ) {
        return rpc_error(RpcErrorCode::InvalidParams);
    }

    if count == limit.saturating_add(1)
        && let Some(marker) = marker
    {
        let JsonValue::Object(object) = &mut result else {
            unreachable!("result should be an object");
        };
        object.insert("limit".to_owned(), JsonValue::Unsigned(u64::from(limit)));
        object.insert(
            "marker".to_owned(),
            JsonValue::String(format!("{},{}", marker, next_hint)),
        );
    }

    let JsonValue::Object(object) = &mut result else {
        unreachable!("result should be an object");
    };
    object.insert(
        "account".to_owned(),
        JsonValue::String(to_base58(account_id)),
    );
    object.insert("lines".to_owned(), JsonValue::Array(items));
    result
}
