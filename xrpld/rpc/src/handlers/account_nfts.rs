//! Narrow `account_nfts` RPC handler slice.
//!
//! This ports the read-only NFT page walk that the current Rust protocol
//! support can represent without inventing application or database runtime.

#![allow(clippy::unnecessary_cast)]

use std::collections::BTreeMap;

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, JsonOptions, JsonValue, STLedgerEntry, STObject, StBase, get_field_by_symbol,
    nft_page_keylet, nft_page_max_keylet, nft_page_min_keylet, parse_base58_account_id, to_base58,
};

use crate::commands::rpc_helpers::read_limit_field;
use crate::handlers::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcRole, RpcStatus,
    lookup_ledger_with_result,
};
use crate::state::tuning::Tuning;
use crate::status::RpcErrorCode;

const NFT_PAGE_MASK_HEX: &str = "0000000000000000000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFF";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountNFTsRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait AccountNFTsSource: LedgerLookupSource {
    fn read_account_root(
        &self,
        ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Option<STLedgerEntry>;

    fn read_nft_page(
        &self,
        ledger: &LedgerLookupLedger,
        page_key: Uint256,
    ) -> Option<STLedgerEntry>;

    fn succ_nft_page(
        &self,
        ledger: &LedgerLookupLedger,
        start: Uint256,
        last_exclusive: Uint256,
    ) -> Option<Uint256>;

    /// Prefetch the next NFT page key while processing current page.
    /// Default implementation eagerly resolves the next key.
    fn prefetch_next_nft_page(
        &self,
        ledger: &LedgerLookupLedger,
        current_key: Uint256,
        last_exclusive: Uint256,
    ) -> Option<(Uint256, Option<STLedgerEntry>)> {
        let next_key = self.succ_nft_page(ledger, current_key, last_exclusive)?;
        let page = self.read_nft_page(ledger, next_key);
        Some((next_key, page))
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

fn nft_page_mask() -> Uint256 {
    Uint256::from_hex(NFT_PAGE_MASK_HEX).expect("NFT page mask should parse")
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

fn parse_marker(params: &JsonValue) -> Result<Option<Uint256>, RpcStatus> {
    let JsonValue::Object(object) = params else {
        return Ok(None);
    };

    let Some(marker) = object.get("marker") else {
        return Ok(None);
    };

    let JsonValue::String(marker) = marker else {
        return Err(RpcStatus::expected_field_error("marker", "string"));
    };

    Uint256::from_hex(marker)
        .map(Some)
        .map_err(|_| RpcStatus::invalid_field_error("marker"))
}

fn decode_nft_flags(id: Uint256) -> u16 {
    u16::from_be_bytes(id.data()[..2].try_into().expect("NFTokenID flag width"))
}

fn decode_nft_transfer_fee(id: Uint256) -> u16 {
    u16::from_be_bytes(id.data()[2..4].try_into().expect("NFTokenID fee width"))
}

fn decode_nft_issuer(id: Uint256) -> AccountID {
    AccountID::from_slice(&id.data()[4..24]).expect("NFTokenID issuer width")
}

fn decode_nft_taxon(id: Uint256) -> u32 {
    let taxon = u32::from_be_bytes(id.data()[24..28].try_into().expect("NFTokenID taxon width"));
    let serial = decode_nft_serial(id);
    taxon ^ (((384_160_001u32.wrapping_mul(serial)).wrapping_add(2_459)) as u32)
}

fn decode_nft_serial(id: Uint256) -> u32 {
    u32::from_be_bytes(
        id.data()[28..32]
            .try_into()
            .expect("NFTokenID serial width"),
    )
}

fn shape_nft_json(nft: &STObject) -> JsonValue {
    let JsonValue::Object(mut object) = nft.json(JsonOptions::NONE) else {
        unreachable!("NFT objects should render as JSON objects");
    };

    let nft_id = nft.get_field_h256(get_field_by_symbol("sfNFTokenID"));
    let issuer = decode_nft_issuer(nft_id);

    object.insert(
        get_field_by_symbol("sfFlags").name().to_owned(),
        JsonValue::Unsigned(u64::from(decode_nft_flags(nft_id))),
    );
    object.insert(
        get_field_by_symbol("sfIssuer").name().to_owned(),
        JsonValue::String(to_base58(issuer)),
    );
    object.insert(
        get_field_by_symbol("sfNFTokenTaxon").name().to_owned(),
        JsonValue::Unsigned(u64::from(decode_nft_taxon(nft_id))),
    );
    object.insert(
        "nft_serial".to_owned(),
        JsonValue::Unsigned(u64::from(decode_nft_serial(nft_id))),
    );

    let transfer_fee = decode_nft_transfer_fee(nft_id);
    if transfer_fee != 0 {
        object.insert(
            get_field_by_symbol("sfTransferFee").name().to_owned(),
            JsonValue::Unsigned(u64::from(transfer_fee)),
        );
    }

    JsonValue::Object(object)
}

fn collect_nft_pages<S: AccountNFTsSource>(
    source: &S,
    ledger: &LedgerLookupLedger,
    account_id: AccountID,
    marker: Uint256,
    limit: u32,
) -> Result<(Vec<JsonValue>, Option<Uint256>), JsonValue> {
    let mut nfts = Vec::new();
    let mut marker_found = marker.is_zero();
    let mut past_marker = marker.is_zero();
    let masked_marker = marker & nft_page_mask();

    let owner = Uint160::from_slice(account_id.data()).expect("account width");
    let first = nft_page_keylet(nft_page_min_keylet(owner), marker);
    let last = nft_page_max_keylet(owner);
    let mut current_key = if marker.is_zero() {
        source.succ_nft_page(ledger, first.key, last.key.next())
    } else if source.read_nft_page(ledger, first.key).is_some() {
        Some(first.key)
    } else {
        source.succ_nft_page(ledger, first.key, last.key.next())
    };
    while let Some(page_key) = current_key {
        let Some(page) = source.read_nft_page(ledger, page_key) else {
            break;
        };

        // Prefetch next page while we process this one — the SHAMap succ()
        // and page read happen eagerly so tree nodes are warm in cache.
        let prefetched_next = source.prefetch_next_nft_page(ledger, page_key, last.key.next());

        let nftokens = page.get_field_array(get_field_by_symbol("sfNFTokens"));
        for nft in nftokens.iter() {
            let nft_id = nft.get_field_h256(get_field_by_symbol("sfNFTokenID"));
            let masked_nft_id = nft_id & nft_page_mask();

            if !past_marker {
                if masked_nft_id < masked_marker {
                    continue;
                }
                if masked_nft_id == masked_marker && nft_id < marker {
                    continue;
                }
                if nft_id == marker {
                    marker_found = true;
                    continue;
                }
            }

            if !marker.is_zero() && !marker_found {
                return Err(crate::commands::rpc_helpers::invalid_field_error("marker"));
            }

            past_marker = true;
            nfts.push(shape_nft_json(nft));

            if nfts.len() as u32 == limit {
                return Ok((nfts, Some(nft_id)));
            }
        }

        // Use prefetched next page (already resolved)
        current_key = prefetched_next.map(|(k, _)| k);
    }

    if !marker.is_zero() && !marker_found {
        return Err(crate::commands::rpc_helpers::invalid_field_error("marker"));
    }

    Ok((nfts, None))
}

pub fn do_account_nfts<S: AccountNFTsSource>(
    request: &AccountNFTsRequest<'_>,
    source: &S,
) -> JsonValue {
    tracing::trace!(target: "rpc", method = "account_nfts", "account_nfts query");
    let account = match parse_account(request.params) {
        Ok(account) => account,
        Err(error) => return error,
    };

    let context = LedgerLookupContext {
        params: request.params,
        source,
        api_version: request.api_version,
        role: request.role,
    };

    let (ledger, result) = match lookup_ledger_with_result(&context) {
        Ok(result) => result,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
    };

    let Some(account_id) = parse_base58_account_id(&account) else {
        return crate::commands::rpc_helpers::rpc_error(RpcErrorCode::ActMalformed);
    };

    if source.read_account_root(&ledger, account_id).is_none() {
        return crate::commands::rpc_helpers::rpc_error(RpcErrorCode::ActNotFound);
    }

    let limit = match read_limit_field(request.params, request.role, Tuning::ACCOUNT_NFTOKENS) {
        Ok(limit) => limit,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
    };

    let marker = match parse_marker(request.params) {
        Ok(marker) => marker.unwrap_or_default(),
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
    };

    let (nfts, next_marker) = match collect_nft_pages(source, &ledger, account_id, marker, limit) {
        Ok(result) => result,
        Err(error) => return error,
    };

    let mut result = result;
    let object = ensure_object(&mut result);
    object.insert("account_nfts".to_owned(), JsonValue::Array(nfts));
    object.insert(
        "account".to_owned(),
        JsonValue::String(to_base58(account_id)),
    );

    if let Some(marker) = next_marker {
        object.insert("limit".to_owned(), JsonValue::Unsigned(u64::from(limit)));
        object.insert("marker".to_owned(), JsonValue::String(marker.to_string()));
    }

    result
}
