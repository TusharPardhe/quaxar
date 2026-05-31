//! Narrow `book_offers` RPC handler slice.
//!
//! This ports the the reference implementation request parsing and result shaping flow without
//! inventing an `Application` or `NetworkOPs` clone. The handler owns the
//! ledger lookup and validation steps, then delegates page shaping to an
//! explicit runtime trait.

#![allow(clippy::too_many_arguments)]

use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{
    AccountID, Asset, Book, Currency, Issue, JsonValue, MPTID, MPTIssue, is_xrp_currency,
    no_account, parse_base58_account_id, to_currency, to_issuer, xrp_account,
};

use crate::commands::rpc_helpers::{
    expected_field_error, invalid_field_error, make_error_message, missing_field_error,
    read_limit_field,
};
use crate::handlers::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcRole, lookup_ledger_with_result,
};
use crate::state::tuning::Tuning;
use crate::status::RpcErrorCode;

const TOO_BUSY_THRESHOLD: u32 = 200;

const TOO_BUSY_CODE: i32 = 9;
const TOO_BUSY_TOKEN: &str = "tooBusy";
const TOO_BUSY_MESSAGE: &str = "The server is too busy to help you now.";

const BAD_MARKET_CODE: i32 = 42;
const BAD_MARKET_TOKEN: &str = "badMarket";
const BAD_MARKET_MESSAGE: &str = "No such market.";

const DOMAIN_MALFORMED_CODE: i32 = 97;
const DOMAIN_MALFORMED_TOKEN: &str = "domainMalformed";

const SRC_CUR_MALFORMED_CODE: i32 = 69;
const SRC_CUR_MALFORMED_TOKEN: &str = "srcCurMalformed";

const DST_AMT_MALFORMED_CODE: i32 = 51;
const DST_AMT_MALFORMED_TOKEN: &str = "dstAmtMalformed";

const SRC_ISR_MALFORMED_CODE: i32 = 70;
const SRC_ISR_MALFORMED_TOKEN: &str = "srcIsrMalformed";

const DST_ISR_MALFORMED_CODE: i32 = 53;
const DST_ISR_MALFORMED_TOKEN: &str = "dstIsrMalformed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookOffersRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait BookOffersSource: LedgerLookupSource {
    fn client_job_count_gt(&self, threshold: u32) -> bool;
}

pub trait BookOffersRuntime {
    fn get_book_page(
        &self,
        ledger: &LedgerLookupLedger,
        book: Book,
        taker: AccountID,
        proof: bool,
        limit: u32,
        marker: JsonValue,
        result: &mut JsonValue,
    );
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

fn rpc_error(code: i32, token: &str, message: impl Into<String>) -> JsonValue {
    let mut error = JsonValue::Object(BTreeMap::new());
    let object = ensure_object(&mut error);
    object.insert("error".to_owned(), JsonValue::String(token.to_owned()));
    object.insert("error_code".to_owned(), JsonValue::Signed(i64::from(code)));
    object.insert(
        "error_message".to_owned(),
        JsonValue::String(message.into()),
    );
    error
}

fn too_busy_error() -> JsonValue {
    rpc_error(TOO_BUSY_CODE, TOO_BUSY_TOKEN, TOO_BUSY_MESSAGE)
}

fn bad_market_error() -> JsonValue {
    rpc_error(BAD_MARKET_CODE, BAD_MARKET_TOKEN, BAD_MARKET_MESSAGE)
}

fn domain_malformed_error() -> JsonValue {
    rpc_error(
        DOMAIN_MALFORMED_CODE,
        DOMAIN_MALFORMED_TOKEN,
        "Unable to parse domain.",
    )
}

fn currency_malformed_error(field: &str, code: i32, token: &str) -> JsonValue {
    rpc_error(
        code,
        token,
        format!("Invalid field '{field}', bad currency."),
    )
}

fn mpt_malformed_error(field: &str, code: i32, token: &str) -> JsonValue {
    rpc_error(code, token, format!("Invalid field '{field}'."))
}

fn issuer_unneeded_error(field: &str, code: i32, token: &str) -> JsonValue {
    rpc_error(
        code,
        token,
        format!("Unneeded field '{field}' for XRP currency specification."),
    )
}

fn issuer_malformed_error(field: &str, code: i32, token: &str, reason: &str) -> JsonValue {
    rpc_error(code, token, format!("Invalid field '{field}', {reason}"))
}

fn object_field_error(field: &str) -> JsonValue {
    make_error_message(
        RpcErrorCode::InvalidParams,
        format!("Invalid field '{field}', not object."),
    )
}

fn parse_taker(params: &JsonValue) -> Result<AccountID, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Ok(no_account());
    };

    let Some(taker) = object.get("taker") else {
        return Ok(no_account());
    };

    let JsonValue::String(taker) = taker else {
        return Err(expected_field_error("taker", "string"));
    };

    parse_base58_account_id(taker).ok_or_else(|| invalid_field_error("taker"))
}

fn parse_domain(params: &JsonValue) -> Result<Option<Uint256>, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Ok(None);
    };

    let Some(domain) = object.get("domain") else {
        return Ok(None);
    };

    let JsonValue::String(domain) = domain else {
        return Err(domain_malformed_error());
    };

    Uint256::from_hex(domain)
        .map(Some)
        .map_err(|_| domain_malformed_error())
}

fn parse_asset_leg(
    params: &JsonValue,
    leg_name: &'static str,
    currency_error_code: i32,
    currency_error_token: &'static str,
    issuer_error_code: i32,
    issuer_error_token: &'static str,
) -> Result<Asset, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Err(missing_field_error(leg_name));
    };

    let Some(leg_value) = object.get(leg_name) else {
        return Err(missing_field_error(leg_name));
    };

    let leg_object = match leg_value {
        JsonValue::Object(object) => Some(object),
        JsonValue::Null => None,
        _ => return Err(object_field_error(leg_name)),
    };

    let mpt_field = format!("{leg_name}.mpt_issuance_id");
    if let Some(mpt_value) = leg_object.and_then(|object| object.get("mpt_issuance_id")) {
        if leg_object
            .is_some_and(|object| object.contains_key("currency") || object.contains_key("issuer"))
        {
            return Err(invalid_field_error(leg_name));
        }
        let JsonValue::String(mpt_id) = mpt_value else {
            return Err(expected_field_error(
                &format!("{leg_name}.currency"),
                "string",
            ));
        };
        return MPTID::from_hex(mpt_id)
            .map(|id| Asset::from(MPTIssue::new(id)))
            .map_err(|_| {
                mpt_malformed_error(&mpt_field, currency_error_code, currency_error_token)
            });
    }

    let currency_field = format!("{leg_name}.currency");
    let currency_text = match leg_object.and_then(|object| object.get("currency")) {
        Some(JsonValue::String(currency)) => currency.clone(),
        Some(_) => return Err(expected_field_error(&currency_field, "string")),
        None => return Err(missing_field_error(&currency_field)),
    };

    let mut currency = Currency::zero();
    if !to_currency(&mut currency, &currency_text) {
        return Err(currency_malformed_error(
            &currency_field,
            currency_error_code,
            currency_error_token,
        ));
    }

    let issuer_field = format!("{leg_name}.issuer");
    let issuer = match leg_object.and_then(|object| object.get("issuer")) {
        None => xrp_account(),
        Some(JsonValue::String(issuer)) => {
            let mut parsed = no_account();
            if !to_issuer(&mut parsed, issuer) {
                return Err(issuer_malformed_error(
                    &issuer_field,
                    issuer_error_code,
                    issuer_error_token,
                    "bad issuer.",
                ));
            }

            if parsed == no_account() {
                return Err(issuer_malformed_error(
                    &issuer_field,
                    issuer_error_code,
                    issuer_error_token,
                    "bad issuer account one.",
                ));
            }

            parsed
        }
        Some(_) => return Err(expected_field_error(&issuer_field, "string")),
    };

    if is_xrp_currency(currency) && issuer != xrp_account() {
        return Err(issuer_unneeded_error(
            &issuer_field,
            issuer_error_code,
            issuer_error_token,
        ));
    }

    if !is_xrp_currency(currency) && issuer == xrp_account() {
        return Err(issuer_malformed_error(
            &issuer_field,
            issuer_error_code,
            issuer_error_token,
            "expected non-XRP issuer.",
        ));
    }

    Ok(Asset::from(Issue::new(currency, issuer)))
}

fn parse_taker_pays(params: &JsonValue) -> Result<Asset, JsonValue> {
    parse_asset_leg(
        params,
        "taker_pays",
        SRC_CUR_MALFORMED_CODE,
        SRC_CUR_MALFORMED_TOKEN,
        SRC_ISR_MALFORMED_CODE,
        SRC_ISR_MALFORMED_TOKEN,
    )
}

fn parse_taker_gets(params: &JsonValue) -> Result<Asset, JsonValue> {
    parse_asset_leg(
        params,
        "taker_gets",
        DST_AMT_MALFORMED_CODE,
        DST_AMT_MALFORMED_TOKEN,
        DST_ISR_MALFORMED_CODE,
        DST_ISR_MALFORMED_TOKEN,
    )
}

fn parse_book_request(
    params: &JsonValue,
    role: RpcRole,
) -> Result<(Book, AccountID, bool, u32, JsonValue), JsonValue> {
    let pay = parse_taker_pays(params)?;
    let get = parse_taker_gets(params)?;
    let taker = parse_taker(params)?;
    let domain = parse_domain(params)?;

    if pay == get {
        return Err(bad_market_error());
    }

    let limit = match read_limit_field(params, role, Tuning::BOOK_OFFERS) {
        Ok(limit) => limit,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return Err(error);
        }
    };

    let proof = matches!(params, JsonValue::Object(object) if object.contains_key("proof"));
    let marker = match params {
        JsonValue::Object(object) => object.get("marker").cloned().unwrap_or(JsonValue::Null),
        _ => JsonValue::Null,
    };

    Ok((Book::new(pay, get, domain), taker, proof, limit, marker))
}

pub fn do_book_offers<S, R>(request: &BookOffersRequest<'_>, source: &S, runtime: &R) -> JsonValue
where
    S: BookOffersSource,
    R: BookOffersRuntime,
{
    tracing::trace!(target: "rpc", method = "book_offers", "book_offers query");
    if source.client_job_count_gt(TOO_BUSY_THRESHOLD) {
        return too_busy_error();
    }

    let context = LedgerLookupContext {
        params: request.params,
        source,
        api_version: request.api_version,
        role: request.role,
    };

    let (ledger, mut result) = match lookup_ledger_with_result(&context) {
        Ok(ledger) => ledger,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
    };

    let (book, taker, proof, limit, marker) = match parse_book_request(request.params, request.role)
    {
        Ok(parsed) => parsed,
        Err(error) => return error,
    };

    runtime.get_book_page(&ledger, book, taker, proof, limit, marker, &mut result);
    result
}
