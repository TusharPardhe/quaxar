//! Read-only NFT offer enumeration shared by `nft_buy_offers` and
//! `nft_sell_offers`.

use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{
    JsonOptions, JsonValue, STLedgerEntry, STVector256, StBase, get_field_by_symbol,
    nft_buy_offers_keylet, nft_offer_keylet, nft_sell_offers_keylet, page_keylet,
};

use crate::commands::rpc_helpers::{
    expected_field_error, invalid_field_error, missing_field_error, rpc_error,
};
use crate::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcRole, lookup_ledger_with_result,
};
use crate::state::tuning::Tuning;
use crate::status::RpcErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NFTOfferKind {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NFTOffersRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait NFTOffersSource: LedgerLookupSource {
    fn read_directory_page(
        &self,
        ledger: &LedgerLookupLedger,
        page_key: Uint256,
    ) -> Option<STLedgerEntry>;

    fn read_nft_offer(
        &self,
        ledger: &LedgerLookupLedger,
        offer_key: Uint256,
    ) -> Option<STLedgerEntry>;
}

fn parse_nft_id(params: &JsonValue) -> Result<Uint256, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Err(missing_field_error("nft_id"));
    };

    let Some(value) = object.get("nft_id") else {
        return Err(missing_field_error("nft_id"));
    };

    let JsonValue::String(text) = value else {
        return Err(invalid_field_error("nft_id"));
    };

    Uint256::from_hex(text).map_err(|_| invalid_field_error("nft_id"))
}

fn parse_marker(params: &JsonValue) -> Result<Option<Uint256>, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Ok(None);
    };

    let Some(value) = object.get("marker") else {
        return Ok(None);
    };

    let JsonValue::String(text) = value else {
        return Err(expected_field_error("marker", "string"));
    };

    Uint256::from_hex(text)
        .map(Some)
        .map_err(|_| rpc_error(RpcErrorCode::InvalidParams))
}

fn append_nft_offer_json(offer: &STLedgerEntry, offers: &mut Vec<JsonValue>) {
    let mut object = BTreeMap::new();
    object.insert(
        "nft_offer_index".to_owned(),
        JsonValue::String(offer.key().to_string()),
    );
    object.insert(
        "flags".to_owned(),
        JsonValue::Unsigned(u64::from(
            offer.get_field_u32(get_field_by_symbol("sfFlags")),
        )),
    );
    object.insert(
        "owner".to_owned(),
        JsonValue::String(protocol::to_base58(
            offer.get_account_id(get_field_by_symbol("sfOwner")),
        )),
    );

    if offer.is_field_present(get_field_by_symbol("sfDestination")) {
        object.insert(
            "destination".to_owned(),
            JsonValue::String(protocol::to_base58(
                offer.get_account_id(get_field_by_symbol("sfDestination")),
            )),
        );
    }

    if offer.is_field_present(get_field_by_symbol("sfExpiration")) {
        object.insert(
            "expiration".to_owned(),
            JsonValue::Unsigned(u64::from(
                offer.get_field_u32(get_field_by_symbol("sfExpiration")),
            )),
        );
    }

    object.insert(
        "amount".to_owned(),
        offer
            .get_field_amount(get_field_by_symbol("sfAmount"))
            .json(JsonOptions::NONE),
    );
    offers.push(JsonValue::Object(object));
}

fn page_indexes(page: &STLedgerEntry) -> STVector256 {
    page.get_field_v256(get_field_by_symbol("sfIndexes"))
}

fn directory_contains_after<S: NFTOffersSource>(
    source: &S,
    ledger: &LedgerLookupLedger,
    directory: protocol::Keylet,
    page_index: u64,
    after: Uint256,
) -> bool {
    let Some(page) = source.read_directory_page(ledger, page_keylet(directory, page_index).key)
    else {
        return false;
    };

    page_indexes(&page)
        .value()
        .iter()
        .any(|entry| *entry == after)
}

fn collect_offer_keys_after<S: NFTOffersSource>(
    source: &S,
    ledger: &LedgerLookupLedger,
    directory: protocol::Keylet,
    after: Uint256,
    hint: u64,
    mut reserve: u32,
) -> Result<Vec<Uint256>, JsonValue> {
    let mut current_index = 0u64;
    let mut found = after.is_zero();
    let mut keys = Vec::new();

    if !after.is_zero() && directory_contains_after(source, ledger, directory, hint, after) {
        current_index = hint;
    }

    loop {
        let Some(page) =
            source.read_directory_page(ledger, page_keylet(directory, current_index).key)
        else {
            return if after.is_zero() || found {
                Ok(keys)
            } else {
                Err(rpc_error(RpcErrorCode::InvalidParams))
            };
        };

        for entry in page_indexes(&page).value().iter().copied() {
            if !found {
                if entry == after {
                    found = true;
                }
                continue;
            }

            keys.push(entry);
            reserve = reserve.saturating_sub(1);
            if reserve == 0 {
                return Ok(keys);
            }
        }

        current_index = page.get_field_u64(get_field_by_symbol("sfIndexNext"));
        if current_index == 0 {
            break;
        }
    }

    if !after.is_zero() && !found {
        return Err(rpc_error(RpcErrorCode::InvalidParams));
    }

    Ok(keys)
}

fn enumerate_nft_offers<S: NFTOffersSource>(
    request: &NFTOffersRequest<'_>,
    source: &S,
    nft_id: Uint256,
    kind: NFTOfferKind,
) -> JsonValue {
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

    let limit = match crate::commands::rpc_helpers::read_limit_field(
        request.params,
        request.role,
        Tuning::NFT_OFFERS,
    ) {
        Ok(value) => value,
        Err(status) => {
            let mut json = JsonValue::Object(BTreeMap::new());
            status.inject(&mut json);
            return json;
        }
    };
    let marker = match parse_marker(request.params) {
        Ok(value) => value,
        Err(error) => return error,
    };

    let directory = match kind {
        NFTOfferKind::Buy => nft_buy_offers_keylet(nft_id),
        NFTOfferKind::Sell => nft_sell_offers_keylet(nft_id),
    };
    if source.read_directory_page(&ledger, directory.key).is_none() {
        return rpc_error(RpcErrorCode::ObjectNotFound);
    }

    let mut offers = Vec::new();
    let mut reserve = limit;
    let mut start_after = Uint256::zero();
    let mut start_hint = 0u64;

    if let Some(marker) = marker {
        let Some(offer) = source.read_nft_offer(&ledger, nft_offer_keylet(marker).key) else {
            return rpc_error(RpcErrorCode::InvalidParams);
        };
        if offer.get_field_h256(get_field_by_symbol("sfNFTokenID")) != nft_id {
            return rpc_error(RpcErrorCode::InvalidParams);
        }
        start_after = marker;
        start_hint = offer.get_field_u64(get_field_by_symbol("sfNFTokenOfferNode"));
        append_nft_offer_json(&offer, &mut offers);
    } else {
        reserve += 1;
    }

    let keys = match collect_offer_keys_after(
        source,
        &ledger,
        directory,
        start_after,
        start_hint,
        reserve,
    ) {
        Ok(keys) => keys,
        Err(error) => return error,
    };

    let mut page_offers = Vec::new();
    for key in keys {
        let Some(offer) = source.read_nft_offer(&ledger, key) else {
            return rpc_error(RpcErrorCode::InvalidParams);
        };
        if offer.get_type() != protocol::LedgerEntryType::NFTokenOffer {
            return rpc_error(RpcErrorCode::InvalidParams);
        }
        page_offers.push(offer);
    }

    let marker_value = if page_offers.len() as u32 == reserve {
        let marker_offer = page_offers
            .pop()
            .expect("reserve-sized page offers must be non-empty");
        Some(marker_offer.key().to_string())
    } else {
        None
    };

    for offer in &page_offers {
        append_nft_offer_json(offer, &mut offers);
    }

    let JsonValue::Object(object) = &mut result else {
        return result;
    };
    object.insert("nft_id".to_owned(), JsonValue::String(nft_id.to_string()));
    object.insert("offers".to_owned(), JsonValue::Array(offers));
    if let Some(marker) = marker_value {
        object.insert("limit".to_owned(), JsonValue::Unsigned(u64::from(limit)));
        object.insert("marker".to_owned(), JsonValue::String(marker));
    }
    result
}

pub fn do_nft_buy_offers<S: NFTOffersSource>(
    request: &NFTOffersRequest<'_>,
    source: &S,
) -> JsonValue {
    let nft_id = match parse_nft_id(request.params) {
        Ok(value) => value,
        Err(error) => return error,
    };
    enumerate_nft_offers(request, source, nft_id, NFTOfferKind::Buy)
}

pub fn do_nft_sell_offers<S: NFTOffersSource>(
    request: &NFTOffersRequest<'_>,
    source: &S,
) -> JsonValue {
    let nft_id = match parse_nft_id(request.params) {
        Ok(value) => value,
        Err(error) => return error,
    };
    enumerate_nft_offers(request, source, nft_id, NFTOfferKind::Sell)
}
