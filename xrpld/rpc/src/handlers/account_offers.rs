//! Read-only `account_offers` RPC slice.

#![allow(clippy::manual_contains)]

use std::{collections::BTreeMap, sync::Arc};

use basics::base_uint::Uint256;
use protocol::{
    AccountID, JsonOptions, JsonValue, LedgerEntryType, NFTokenOffer, Offer, PayChannel,
    RippleState, STAmount, STLedgerEntry, StBase, get_field_by_symbol, no_issue,
    parse_base58_account_id, quality_from_key, signers_keylet, to_base58,
};

use crate::commands::rpc_helpers::read_limit_field;
use crate::handlers::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcRole, RpcStatus,
    lookup_ledger_with_result,
};
use crate::state::tuning::Tuning;
use crate::status::RpcErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountOffersRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait AccountOffersSource: LedgerLookupSource {
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

    /// Batch-read multiple child entries from a directory page.
    /// Default implementation falls back to sequential reads.
    /// Optimized implementations can use nodestore fetch_batch.
    fn read_child_entries_batch(
        &self,
        ledger: &LedgerLookupLedger,
        entries: &[Uint256],
    ) -> Vec<Option<STLedgerEntry>> {
        entries.iter().map(|e| self.read_child_entry(ledger, *e)).collect()
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

fn make_error(code: RpcErrorCode) -> JsonValue {
    let mut json = JsonValue::Object(BTreeMap::new());
    crate::commands::rpc_helpers::inject_error(code, &mut json);
    json
}

fn parse_account(params: &JsonValue) -> Result<String, RpcStatus> {
    let JsonValue::Object(object) = params else {
        return Err(RpcStatus::missing_field_error("account"));
    };

    let Some(account) = object.get("account") else {
        return Err(RpcStatus::missing_field_error("account"));
    };

    let JsonValue::String(account) = account else {
        return Err(RpcStatus::invalid_field_error("account"));
    };

    Ok(account.clone())
}

fn amount_text_from_quality(rate: u64) -> String {
    if rate == 0 {
        return STAmount::new_with_asset(protocol::sf_generic(), no_issue(), 0, 0, false).text();
    }

    let mantissa = rate & !(255u64 << 56);
    let exponent = i32::try_from((rate >> 56) as i64).expect("quality exponent fits i32") - 100;
    STAmount::new_with_asset(
        protocol::sf_generic(),
        no_issue(),
        mantissa,
        exponent,
        false,
    )
    .text()
}

fn append_offer_json(offer: &Offer) -> JsonValue {
    let mut object = BTreeMap::new();
    object.insert(
        "taker_pays".to_owned(),
        offer.get_taker_pays().json(JsonOptions::NONE),
    );
    object.insert(
        "taker_gets".to_owned(),
        offer.get_taker_gets().json(JsonOptions::NONE),
    );
    object.insert(
        "seq".to_owned(),
        JsonValue::Unsigned(u64::from(offer.get_sequence())),
    );
    object.insert(
        "flags".to_owned(),
        JsonValue::Unsigned(u64::from(offer.get_flags())),
    );
    object.insert(
        "quality".to_owned(),
        JsonValue::String(amount_text_from_quality(quality_from_key(
            offer.get_book_directory(),
        ))),
    );
    if let Some(expiration) = offer.get_expiration() {
        object.insert(
            "expiration".to_owned(),
            JsonValue::Unsigned(u64::from(expiration)),
        );
    }

    JsonValue::Object(object)
}

fn get_start_hint(sle: &STLedgerEntry, account_id: AccountID) -> u64 {
    match sle.get_type() {
        LedgerEntryType::RippleState => {
            let ripple = RippleState::new(Arc::new(sle.clone()))
                .expect("RippleState marker entries should wrap cleanly");
            if ripple.get_low_limit().issue().account == account_id {
                return ripple.get_low_node().unwrap_or_default();
            }
            if ripple.get_high_limit().issue().account == account_id {
                return ripple.get_high_node().unwrap_or_default();
            }
            0
        }
        LedgerEntryType::Offer => Offer::new(Arc::new(sle.clone()))
            .expect("Offer marker entries should wrap cleanly")
            .get_owner_node(),
        LedgerEntryType::PayChannel => PayChannel::new(Arc::new(sle.clone()))
            .expect("PayChannel marker entries should wrap cleanly")
            .get_owner_node(),
        LedgerEntryType::NFTokenOffer => NFTokenOffer::new(Arc::new(sle.clone()))
            .expect("NFTokenOffer marker entries should wrap cleanly")
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
            let ripple = RippleState::new(Arc::new(sle.clone()))
                .expect("RippleState ownership markers should wrap cleanly");
            ripple.get_low_limit().issue().account == account_id
                || ripple.get_high_limit().issue().account == account_id
        }
        LedgerEntryType::Offer => {
            let offer = Offer::new(Arc::new(sle.clone()))
                .expect("Offer ownership markers should wrap cleanly");
            offer.get_account() == account_id
        }
        LedgerEntryType::PayChannel => {
            let channel = PayChannel::new(Arc::new(sle.clone()))
                .expect("PayChannel ownership markers should wrap cleanly");
            channel.get_account() == account_id || channel.get_destination() == account_id
        }
        LedgerEntryType::NFTokenOffer => {
            let nft_offer = NFTokenOffer::new(Arc::new(sle.clone()))
                .expect("NFTokenOffer ownership markers should wrap cleanly");
            nft_offer.get_owner() == account_id
        }
        LedgerEntryType::SignerList => {
            *sle.key()
                == signers_keylet(
                    basics::base_uint::Uint160::from_slice(account_id.data())
                        .expect("account width"),
                )
                .key
        }
        _ if sle.is_field_present(get_field_by_symbol("sfAccount")) => {
            sle.get_account_id(get_field_by_symbol("sfAccount")) == account_id
                || (sle.is_field_present(get_field_by_symbol("sfDestination"))
                    && sle.get_account_id(get_field_by_symbol("sfDestination")) == account_id)
        }
        _ => false,
    }
}

fn directory_contains_after<S: AccountOffersSource>(
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
    S: AccountOffersSource,
    F: FnMut(&STLedgerEntry) -> bool,
{
    let root_page = 0u64;
    let mut current_index = root_page;

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

            let entries: Vec<Uint256> = owner_dir
                .get_field_v256(get_field_by_symbol("sfIndexes"))
                .value()
                .to_vec();

            // Batch-fetch all entries in this page at once
            let sles = source.read_child_entries_batch(ledger, &entries);

            for (entry, sle_opt) in entries.iter().zip(sles.into_iter()) {
                if !found {
                    if *entry == after {
                        found = true;
                    }
                    continue;
                }

                let Some(sle) = sle_opt else {
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

        let entries: Vec<Uint256> = owner_dir
            .get_field_v256(get_field_by_symbol("sfIndexes"))
            .value()
            .to_vec();

        // Batch-fetch all entries in this page at once
        let sles = source.read_child_entries_batch(ledger, &entries);

        for sle_opt in sles {
            let Some(sle) = sle_opt else {
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

fn has_more_items_after<S: AccountOffersSource>(
    source: &S,
    ledger: &LedgerLookupLedger,
    account_id: AccountID,
    after: Uint256,
    hint: u64,
) -> bool {
    let mut found = false;
    let ok = for_each_item_after(source, ledger, account_id, after, hint, 1, |_sle| {
        found = true;
        true
    });
    ok && found
}

pub fn do_account_offers<S: AccountOffersSource>(
    request: &AccountOffersRequest<'_>,
    source: &S,
) -> JsonValue {
    tracing::trace!(target: "rpc", method = "account_offers", "account_offers query");
    let account_text = match parse_account(request.params) {
        Ok(account) => account,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
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
        return make_error(RpcErrorCode::ActMalformed);
    };

    ensure_object(&mut result).insert(
        "account".to_owned(),
        JsonValue::String(to_base58(account_id)),
    );

    if source.read_account_root(&ledger, account_id).is_none() {
        return make_error(RpcErrorCode::ActNotFound);
    }

    let limit = match read_limit_field(request.params, request.role, Tuning::ACCOUNT_OFFERS) {
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
    result_object.insert("offers".to_owned(), JsonValue::Array(Vec::new()));

    let mut start_after = Uint256::zero();
    let mut start_hint = 0u64;
    let marker_set =
        matches!(request.params, JsonValue::Object(object) if object.contains_key("marker"));
    if marker_set {
        let JsonValue::Object(object) = request.params else {
            unreachable!("marker parsing requires an object");
        };

        let Some(marker_value) = object.get("marker") else {
            return make_error(RpcErrorCode::InvalidParams);
        };

        let JsonValue::String(marker_text) = marker_value else {
            return crate::commands::rpc_helpers::expected_field_error("marker", "string");
        };

        let mut parts = marker_text.splitn(3, ',');
        let Some(first) = parts.next() else {
            return crate::commands::rpc_helpers::invalid_field_error("marker");
        };
        if !start_after.parse_hex(first) {
            return crate::commands::rpc_helpers::invalid_field_error("marker");
        }

        let Some(second) = parts.next() else {
            return crate::commands::rpc_helpers::invalid_field_error("marker");
        };
        match second.parse::<u64>() {
            Ok(hint) => start_hint = hint,
            Err(_) => return crate::commands::rpc_helpers::invalid_field_error("marker"),
        }

        let Some(marker_sle) = source.read_child_entry(&ledger, start_after) else {
            return make_error(RpcErrorCode::InvalidParams);
        };
        if !is_related_to_account(&marker_sle, account_id) {
            return make_error(RpcErrorCode::InvalidParams);
        }
    }

    let mut count = 0u32;
    let mut marker: Option<Uint256> = None;
    let mut next_hint = 0u64;
    let mut offers = Vec::new();

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

            if count <= limit && sle.get_type() == LedgerEntryType::Offer {
                let offer = Offer::new(Arc::new(sle.clone()))
                    .expect("Offer traversal entries should wrap cleanly");
                offers.push(append_offer_json(&offer));
            }

            true
        },
    ) {
        return make_error(RpcErrorCode::InvalidParams);
    }

    let should_emit_marker = if let Some(marker_key) = marker {
        count > limit
            || (count == limit
                && has_more_items_after(source, &ledger, account_id, marker_key, next_hint))
    } else {
        false
    };

    if should_emit_marker && let Some(marker) = marker {
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
    object.insert("offers".to_owned(), JsonValue::Array(offers));
    result
}
