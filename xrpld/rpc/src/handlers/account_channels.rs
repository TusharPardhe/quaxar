//! Narrow `account_channels` RPC handler slice.
//!
//! This ports the the reference implementation account-channel traversal shape onto the
//! existing Rust ledger and keylet helpers, without inventing an
//! application/runtime seam. This keeps the owner-directory walk,
//! marker validation, and pay-channel field shaping aligned with reference,

#![allow(clippy::manual_contains, clippy::question_mark)]
//! including account-public base58 rendering for `public_key`.

use std::{collections::BTreeMap, sync::Arc};

use basics::base_uint::{Uint160, Uint256};
use basics::str_hex::str_hex;
use protocol::{
    AccountID, JsonValue, LedgerEntryType, NFTokenOffer, PayChannel, PublicKey, RippleState,
    STLedgerEntry, StBase, TokenType, encode_base58_token, get_field_by_symbol,
    parse_base58_account_id, to_base58,
};

use crate::commands::rpc_helpers::{
    expected_field_error, invalid_field_error, read_limit_field, rpc_error,
};
use crate::handlers::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcRole, RpcStatus,
    lookup_ledger_with_result,
};
use crate::state::tuning::Tuning;
use crate::status::RpcErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountChannelsRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait AccountChannelsSource: LedgerLookupSource {
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

fn destination_string(params: &JsonValue) -> Option<String> {
    let JsonValue::Object(object) = params else {
        return None;
    };

    let Some(value) = object.get("destination_account") else {
        return None;
    };

    match value {
        JsonValue::String(text) => Some(text.clone()),
        JsonValue::Null => Some(String::new()),
        JsonValue::Bool(flag) => Some(flag.to_string()),
        JsonValue::Signed(value) => Some(value.to_string()),
        JsonValue::Unsigned(value) => Some(value.to_string()),
        JsonValue::Array(_) | JsonValue::Object(_) => Some(String::new()),
    }
}

fn json_to_error(status: RpcStatus) -> JsonValue {
    let mut error = JsonValue::Object(BTreeMap::new());
    status.inject(&mut error);
    error
}

fn get_start_hint(sle: &STLedgerEntry, account_id: AccountID) -> u64 {
    match sle.get_type() {
        LedgerEntryType::RippleState => {
            let ripple_state = RippleState::new(Arc::new(sle.clone()))
                .expect("RippleState marker entries should wrap cleanly");
            if ripple_state.get_low_limit().issue().account == account_id {
                return ripple_state.get_low_node().unwrap_or_default();
            }
            if ripple_state.get_high_limit().issue().account == account_id {
                return ripple_state.get_high_node().unwrap_or_default();
            }
            0
        }
        LedgerEntryType::PayChannel => PayChannel::new(Arc::new(sle.clone()))
            .expect("PayChannel marker entries should wrap cleanly")
            .get_owner_node(),
        _ => {
            if !sle.is_field_present(get_field_by_symbol("sfOwnerNode")) {
                return 0;
            }

            sle.get_field_u64(get_field_by_symbol("sfOwnerNode"))
        }
    }
}

fn is_related_to_account(sle: &STLedgerEntry, account_id: AccountID) -> bool {
    match sle.get_type() {
        LedgerEntryType::RippleState => {
            let ripple_state = RippleState::new(Arc::new(sle.clone()))
                .expect("RippleState ownership markers should wrap cleanly");
            ripple_state.get_low_limit().issue().account == account_id
                || ripple_state.get_high_limit().issue().account == account_id
        }
        LedgerEntryType::PayChannel => {
            let pay_channel = PayChannel::new(Arc::new(sle.clone()))
                .expect("PayChannel ownership markers should wrap cleanly");
            pay_channel.get_account() == account_id || pay_channel.get_destination() == account_id
        }
        LedgerEntryType::SignerList => {
            *sle.key()
                == protocol::signers_keylet(
                    Uint160::from_slice(account_id.data()).expect("account width"),
                )
                .key
        }
        LedgerEntryType::NFTokenOffer => {
            let nft_offer = NFTokenOffer::new(Arc::new(sle.clone()))
                .expect("NFTokenOffer ownership markers should wrap cleanly");
            nft_offer.get_owner() == account_id
        }
        _ if sle.is_field_present(get_field_by_symbol("sfAccount")) => {
            sle.get_account_id(get_field_by_symbol("sfAccount")) == account_id
                || (sle.is_field_present(get_field_by_symbol("sfDestination"))
                    && sle.get_account_id(get_field_by_symbol("sfDestination")) == account_id)
        }
        _ => false,
    }
}

fn format_channel_json(channel: &PayChannel) -> JsonValue {
    let mut object = BTreeMap::new();
    object.insert(
        "channel_id".to_owned(),
        JsonValue::String(channel.as_st_ledger_entry().key().to_string()),
    );
    object.insert(
        "account".to_owned(),
        JsonValue::String(to_base58(channel.get_account())),
    );
    object.insert(
        "destination_account".to_owned(),
        JsonValue::String(to_base58(channel.get_destination())),
    );
    object.insert(
        "amount".to_owned(),
        JsonValue::String(channel.get_amount().text()),
    );
    object.insert(
        "balance".to_owned(),
        JsonValue::String(channel.get_balance().text()),
    );

    let public_key = channel.get_public_key();
    if let Ok(public_key) = PublicKey::from_slice(&public_key) {
        object.insert(
            "public_key".to_owned(),
            JsonValue::String(encode_base58_token(
                TokenType::AccountPublic,
                public_key.as_bytes(),
            )),
        );
        object.insert(
            "public_key_hex".to_owned(),
            JsonValue::String(str_hex(public_key.as_bytes())),
        );
    }

    object.insert(
        "settle_delay".to_owned(),
        JsonValue::Unsigned(u64::from(channel.get_settle_delay())),
    );

    if let Some(expiration) = channel.get_expiration() {
        object.insert(
            "expiration".to_owned(),
            JsonValue::Unsigned(u64::from(expiration)),
        );
    }
    if let Some(cancel_after) = channel.get_cancel_after() {
        object.insert(
            "cancel_after".to_owned(),
            JsonValue::Unsigned(u64::from(cancel_after)),
        );
    }
    if let Some(source_tag) = channel.get_source_tag() {
        object.insert(
            "source_tag".to_owned(),
            JsonValue::Unsigned(u64::from(source_tag)),
        );
    }
    if let Some(destination_tag) = channel.get_destination_tag() {
        object.insert(
            "destination_tag".to_owned(),
            JsonValue::Unsigned(u64::from(destination_tag)),
        );
    }

    JsonValue::Object(object)
}

fn directory_contains_after<S: AccountChannelsSource>(
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
        .iter()
        .any(|entry| *entry == after)
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
    S: AccountChannelsSource,
    F: FnMut(&STLedgerEntry) -> bool,
{
    let mut current_index = 0u64;

    if after != Uint256::zero() {
        let hint_index = hint;
        if directory_contains_after(source, ledger, account_id, hint_index, after) {
            current_index = hint_index;
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

                let Some(sle) = source.read_ledger_entry(ledger, entry) else {
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
            let Some(sle) = source.read_ledger_entry(ledger, entry) else {
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

fn parse_marker(
    source: &impl AccountChannelsSource,
    ledger: &LedgerLookupLedger,
    account_id: AccountID,
    params: &JsonValue,
) -> Result<(Uint256, u64), JsonValue> {
    let JsonValue::Object(object) = params else {
        return Ok((Uint256::zero(), 0));
    };

    let Some(marker) = object.get("marker") else {
        return Ok((Uint256::zero(), 0));
    };

    let JsonValue::String(marker) = marker else {
        return Err(expected_field_error("marker", "string"));
    };

    let mut parts = marker.splitn(3, ',');
    let Some(first) = parts.next() else {
        return Err(rpc_error(RpcErrorCode::InvalidParams));
    };
    let Some(second) = parts.next() else {
        return Err(rpc_error(RpcErrorCode::InvalidParams));
    };

    let Ok(after) = Uint256::from_hex(first) else {
        return Err(rpc_error(RpcErrorCode::InvalidParams));
    };

    let Ok(hint) = second.parse::<u64>() else {
        return Err(rpc_error(RpcErrorCode::InvalidParams));
    };

    let Some(marker_sle) = source.read_ledger_entry(ledger, after) else {
        return Err(rpc_error(RpcErrorCode::InvalidParams));
    };

    if !is_related_to_account(&marker_sle, account_id) {
        return Err(rpc_error(RpcErrorCode::InvalidParams));
    }

    Ok((after, hint))
}

pub fn do_account_channels<S: AccountChannelsSource>(
    request: &AccountChannelsRequest<'_>,
    source: &S,
) -> JsonValue {
    tracing::trace!(target: "rpc", method = "account_channels", "account_channels query");
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
        Err(status) => return json_to_error(status),
    };

    let Some(account_id) = parse_base58_account_id(&account_text) else {
        return rpc_error(RpcErrorCode::ActMalformed);
    };

    if source.read_account_root(&ledger, account_id).is_none() {
        return rpc_error(RpcErrorCode::ActNotFound);
    }

    let destination_text = destination_string(request.params);
    let destination_id = match destination_text {
        None => None,
        Some(text) if text.is_empty() => None,
        Some(text) => match parse_base58_account_id(&text) {
            Some(account) => Some(account),
            None => return rpc_error(RpcErrorCode::ActMalformed),
        },
    };

    let limit = match read_limit_field(request.params, request.role, Tuning::ACCOUNT_CHANNELS) {
        Ok(limit) => limit,
        Err(status) => return json_to_error(status),
    };

    let (start_after, start_hint) = match parse_marker(source, &ledger, account_id, request.params)
    {
        Ok(marker) => marker,
        Err(error) => return error,
    };

    let mut count = 0u32;
    let mut marker: Option<Uint256> = None;
    let mut next_hint = 0u64;
    let mut channels = Vec::new();

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

            if count <= limit
                && sle.get_type() == LedgerEntryType::PayChannel
                && sle.get_account_id(get_field_by_symbol("sfAccount")) == account_id
                && destination_id
                    .map(|destination| {
                        sle.get_account_id(get_field_by_symbol("sfDestination")) == destination
                    })
                    .unwrap_or(true)
            {
                let channel = PayChannel::new(Arc::new(sle.clone()))
                    .expect("PayChannel traversal entries should wrap cleanly");
                channels.push(format_channel_json(&channel));
            }

            true
        },
    ) {
        return rpc_error(RpcErrorCode::InvalidParams);
    }

    if count == limit + 1
        && let Some(marker) = marker
    {
        let object = ensure_object(&mut result);
        object.insert("limit".to_owned(), JsonValue::Unsigned(u64::from(limit)));
        object.insert(
            "marker".to_owned(),
            JsonValue::String(format!("{marker},{next_hint}")),
        );
    }

    let object = ensure_object(&mut result);
    object.insert(
        "account".to_owned(),
        JsonValue::String(to_base58(account_id)),
    );
    object.insert("channels".to_owned(), JsonValue::Array(channels));

    result
}
