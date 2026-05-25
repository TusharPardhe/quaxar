//! Narrow `amm_info` RPC handler slice.
//!
//! This ports the the reference implementation parameter validation and the AMM-entry shaping
//! that the landed Rust protocol and ledger helpers can support honestly. It
//! keeps the AMM selector, account/root validation, vote-slot shaping,
//! auction-slot shaping, and freeze-flag reporting explicit while leaving the
//! live pool balance math to the still-unported ledger surface.

use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{
    AccountID, Asset, Issue, JsonOptions, JsonValue, STArray, STLedgerEntry, STObject, StBase, amm,
    get_field_by_symbol, is_xrp_currency, issue_from_json, line, parse_base58_account_id,
    to_base58,
};

use crate::commands::rpc_helpers::rpc_error;
use crate::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcRole, lookup_ledger_with_result,
};
use crate::status::RpcErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmmInfoRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait AmmInfoSource: LedgerLookupSource {
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

fn invalid_parameters(params: &JsonValue) -> bool {
    let JsonValue::Object(object) = params else {
        return true;
    };

    let has_asset = object.contains_key("asset");
    let has_asset2 = object.contains_key("asset2");
    let has_amm_account = object.contains_key("amm_account");
    (has_asset != has_asset2) || (has_asset == has_amm_account)
}

fn parse_amm_account(value: &JsonValue) -> Option<AccountID> {
    parse_base58_account_id(&json_value_as_string(value))
}

fn parse_issue(value: &JsonValue) -> Result<Issue, JsonValue> {
    issue_from_json(value).map_err(|_| rpc_error(RpcErrorCode::IssueMalformed))
}

fn read_amm_entry<S: AmmInfoSource>(
    source: &S,
    ledger: &LedgerLookupLedger,
    entry_index: Uint256,
) -> Result<STLedgerEntry, JsonValue> {
    source
        .read_ledger_entry(ledger, entry_index)
        .ok_or_else(|| rpc_error(RpcErrorCode::ActNotFound))
}

fn parse_assets(params: &JsonValue) -> Result<(Option<Issue>, Option<Issue>), JsonValue> {
    let JsonValue::Object(object) = params else {
        return Ok((None, None));
    };

    let issue1 = if object.contains_key("asset") {
        Some(parse_issue(
            object.get("asset").expect("asset key should be present"),
        )?)
    } else {
        None
    };
    let issue2 = if object.contains_key("asset2") {
        Some(parse_issue(
            object.get("asset2").expect("asset2 key should be present"),
        )?)
    } else {
        None
    };
    Ok((issue1, issue2))
}

fn shape_vote_slots(vote_slots: &STArray) -> JsonValue {
    let mut output = Vec::with_capacity(vote_slots.len());
    for vote in vote_slots.iter() {
        let mut object = BTreeMap::new();
        object.insert(
            "account".to_owned(),
            JsonValue::String(to_base58(
                vote.get_account_id(get_field_by_symbol("sfAccount")),
            )),
        );
        object.insert(
            "trading_fee".to_owned(),
            JsonValue::Unsigned(u64::from(
                vote.get_field_u16(get_field_by_symbol("sfTradingFee")),
            )),
        );
        object.insert(
            "vote_weight".to_owned(),
            JsonValue::Unsigned(u64::from(
                vote.get_field_u32(get_field_by_symbol("sfVoteWeight")),
            )),
        );
        output.push(JsonValue::Object(object));
    }
    JsonValue::Array(output)
}

fn shape_auction_slot(slot: &STObject) -> JsonValue {
    let mut object = BTreeMap::new();
    object.insert(
        "account".to_owned(),
        JsonValue::String(to_base58(
            slot.get_account_id(get_field_by_symbol("sfAccount")),
        )),
    );
    object.insert(
        "discounted_fee".to_owned(),
        JsonValue::Unsigned(u64::from(
            slot.get_field_u16(get_field_by_symbol("sfDiscountedFee")),
        )),
    );
    object.insert(
        "expiration".to_owned(),
        JsonValue::Unsigned(u64::from(
            slot.get_field_u32(get_field_by_symbol("sfExpiration")),
        )),
    );
    object.insert(
        "price".to_owned(),
        slot.get_field_amount(get_field_by_symbol("sfPrice"))
            .json(protocol::JsonOptions::NONE),
    );

    if slot.is_field_present(get_field_by_symbol("sfAuthAccounts")) {
        let mut auth_accounts = Vec::new();
        for auth in slot
            .get_field_array(get_field_by_symbol("sfAuthAccounts"))
            .iter()
        {
            let mut auth_object = BTreeMap::new();
            auth_object.insert(
                "account".to_owned(),
                JsonValue::String(to_base58(
                    auth.get_account_id(get_field_by_symbol("sfAccount")),
                )),
            );
            auth_accounts.push(JsonValue::Object(auth_object));
        }
        object.insert("auth_accounts".to_owned(), JsonValue::Array(auth_accounts));
    }

    JsonValue::Object(object)
}

fn shape_amm_json<S: AmmInfoSource>(
    source: &S,
    ledger: &LedgerLookupLedger,
    amm_entry: &STLedgerEntry,
    issue1: Issue,
    issue2: Issue,
) -> JsonValue {
    let amm_account_id = amm_entry.get_account_id(get_field_by_symbol("sfAccount"));
    let mut object = BTreeMap::new();

    object.insert(
        "account".to_owned(),
        JsonValue::String(to_base58(amm_account_id)),
    );
    object.insert(
        "trading_fee".to_owned(),
        JsonValue::Unsigned(u64::from(
            amm_entry.get_field_u16(get_field_by_symbol("sfTradingFee")),
        )),
    );
    object.insert(
        "lp_token".to_owned(),
        amm_entry
            .get_field_amount(get_field_by_symbol("sfLPTokenBalance"))
            .json(JsonOptions::NONE),
    );

    if amm_entry.is_field_present(get_field_by_symbol("sfVoteSlots")) {
        let vote_slots = amm_entry.get_field_array(get_field_by_symbol("sfVoteSlots"));
        if !vote_slots.is_empty() {
            object.insert("vote_slots".to_owned(), shape_vote_slots(&vote_slots));
        }
    }

    if amm_entry.is_field_present(get_field_by_symbol("sfAuctionSlot")) {
        let slot = amm_entry.get_field_object(get_field_by_symbol("sfAuctionSlot"));
        if slot.is_field_present(get_field_by_symbol("sfAccount")) {
            object.insert("auction_slot".to_owned(), shape_auction_slot(&slot));
        }
    }

    if !is_xrp_currency(issue1.currency) {
        let frozen = is_frozen_like(source, ledger, amm_account_id, issue1);
        object.insert("asset_frozen".to_owned(), JsonValue::Bool(frozen));
    }
    if !is_xrp_currency(issue2.currency) {
        let frozen = is_frozen_like(source, ledger, amm_account_id, issue2);
        object.insert("asset2_frozen".to_owned(), JsonValue::Bool(frozen));
    }

    JsonValue::Object(object)
}

fn is_frozen_like<S: AmmInfoSource>(
    source: &S,
    ledger: &LedgerLookupLedger,
    amm_account: AccountID,
    issue: Issue,
) -> bool {
    if is_xrp_currency(issue.currency) {
        return false;
    }

    if source
        .read_account_root(ledger, issue.account)
        .is_some_and(|root| root.is_flag(protocol::lsfGlobalFreeze))
    {
        return true;
    }

    if issue.account == amm_account {
        return false;
    }

    let Some(line_entry) =
        source.read_ledger_entry(ledger, line(amm_account, issue.account, issue.currency).key)
    else {
        return false;
    };

    line_entry.is_flag(if issue.account > amm_account {
        protocol::lsfHighFreeze
    } else {
        protocol::lsfLowFreeze
    })
}

pub fn do_amm_info<S: AmmInfoSource>(request: &AmmInfoRequest<'_>, source: &S) -> JsonValue {
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

    if request.api_version < 3 && invalid_parameters(request.params) {
        return rpc_error(RpcErrorCode::InvalidParams);
    }

    let (asset1, asset2) = match parse_assets(request.params) {
        Ok(assets) => assets,
        Err(error) => return error,
    };

    let JsonValue::Object(object) = request.params else {
        return rpc_error(RpcErrorCode::InvalidParams);
    };

    let mut amm_account = None;
    if let Some(value) = object.get("amm_account") {
        let Some(parsed) = parse_amm_account(value) else {
            return rpc_error(RpcErrorCode::ActMalformed);
        };
        if source.read_account_root(&ledger, parsed).is_none() {
            return rpc_error(RpcErrorCode::ActMalformed);
        }
        amm_account = Some(parsed);
    }

    if let Some(value) = object.get("account") {
        let Some(parsed) = parse_amm_account(value) else {
            return rpc_error(RpcErrorCode::ActMalformed);
        };
        if source.read_account_root(&ledger, parsed).is_none() {
            return rpc_error(RpcErrorCode::ActMalformed);
        }
    }

    if request.api_version >= 3 && invalid_parameters(request.params) {
        return rpc_error(RpcErrorCode::InvalidParams);
    }

    let amm_key = if let (Some(issue1), Some(issue2)) = (asset1, asset2) {
        amm(Asset::from(issue1), Asset::from(issue2)).key
    } else if let Some(amm_account) = amm_account {
        let account_root = match source.read_account_root(&ledger, amm_account) {
            Some(root) => root,
            None => return rpc_error(RpcErrorCode::ActMalformed),
        };
        let amm_id = account_root.get_field_h256(get_field_by_symbol("sfAMMID"));
        if amm_id.is_zero() {
            return rpc_error(RpcErrorCode::ActNotFound);
        }
        amm_id
    } else {
        return rpc_error(RpcErrorCode::InvalidParams);
    };

    let amm_entry = match read_amm_entry(source, &ledger, amm_key) {
        Ok(entry) => entry,
        Err(error) => return error,
    };

    let (issue1, issue2) = if let (Some(issue1), Some(issue2)) = (asset1, asset2) {
        (issue1, issue2)
    } else {
        let issue1 = match amm_entry
            .get_field_issue(get_field_by_symbol("sfAsset"))
            .asset()
        {
            Asset::Issue(issue) => issue,
            Asset::MPTIssue(_) => return rpc_error(RpcErrorCode::InvalidParams),
        };
        let issue2 = match amm_entry
            .get_field_issue(get_field_by_symbol("sfAsset2"))
            .asset()
        {
            Asset::Issue(issue) => issue,
            Asset::MPTIssue(_) => return rpc_error(RpcErrorCode::InvalidParams),
        };
        (issue1, issue2)
    };

    let amm_json = shape_amm_json(source, &ledger, &amm_entry, issue1, issue2);
    let object = ensure_object(&mut result);
    object.insert("amm".to_owned(), amm_json);
    result
}
