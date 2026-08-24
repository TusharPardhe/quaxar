//! Pathfinding request parser and source seam.

use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{
    AccountID, Asset, Currency, Issue, JsonValue, STAmount, STPathSet, asset_from_json,
    bad_currency, get_or_throw, is_bad_asset, is_consistent, is_xrp_currency, no_currency,
    parse_base58_account_id, to_currency, xrp_account,
};

use quaxar_core::{RpcErrorCode, Status};

#[derive(Debug, Clone, PartialEq)]
pub struct PathFinderRequest {
    pub source_account: String,
    pub destination_account: String,
    pub destination_amount: JsonValue,
    pub send_max: Option<JsonValue>,
    pub source_account_id: AccountID,
    pub destination_account_id: AccountID,
    pub parsed_destination_amount: STAmount,
    pub parsed_send_max: Option<STAmount>,
    pub source_assets: Vec<Asset>,
    pub domain: Option<Uint256>,
    pub convert_all: bool,
    pub id: Option<JsonValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PathFinderResult {
    pub alternatives: JsonValue,
    pub ledger_hash: Option<String>,
    pub ledger_index: Option<u32>,
    pub destination_currencies: Vec<String>,
    pub validated: Option<bool>,
    pub path_context: BTreeMap<Asset, STPathSet>,
}

impl PathFinderResult {
    pub fn alternatives(alternatives: JsonValue) -> Self {
        Self {
            alternatives,
            ledger_hash: None,
            ledger_index: None,
            destination_currencies: Vec::new(),
            validated: None,
            path_context: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathFindTuning {
    pub old: u32,
    pub search: u32,
    pub fast: u32,
    pub max: u32,
}

impl Default for PathFindTuning {
    fn default() -> Self {
        Self {
            old: 2,
            search: 2,
            fast: 2,
            max: 3,
        }
    }
}

pub trait PathFinderSource {
    fn path_find_tuning(&self) -> PathFindTuning {
        PathFindTuning::default()
    }

    fn find_paths(
        &self,
        request: &PathFinderRequest,
        params: &JsonValue,
        search_level: u32,
        is_legacy: bool,
        api_version: u32,
        previous_paths: &BTreeMap<Asset, STPathSet>,
    ) -> Result<PathFinderResult, Status>;
}

fn parse_amount(value: &JsonValue) -> Option<STAmount> {
    let field = protocol::get_field_by_symbol("sfAmount");
    get_or_throw::<STAmount>(
        &JsonValue::Object(BTreeMap::from([(field.name().to_owned(), value.clone())])),
        field,
    )
    .ok()
}

fn is_convert_all(amount: &STAmount) -> bool {
    amount.signum() == -1 && amount.mantissa() == 1 && amount.exponent() == 0
}

fn valid_path_asset(asset: Asset) -> bool {
    !is_bad_asset(asset)
        && match asset {
            Asset::Issue(issue) => is_consistent(issue),
            Asset::MPTIssue(_) => true,
        }
}

fn parse_source_asset(value: &JsonValue, source: AccountID) -> Result<Asset, RpcErrorCode> {
    let JsonValue::Object(object) = value else {
        return Err(RpcErrorCode::SrcCurMalformed);
    };

    if object.contains_key("mpt_issuance_id") {
        if object.contains_key("currency") || object.contains_key("issuer") {
            return Err(RpcErrorCode::SrcCurMalformed);
        }
        return asset_from_json(value).map_err(|_| RpcErrorCode::SrcCurMalformed);
    }

    let Some(JsonValue::String(code)) = object.get("currency") else {
        return Err(RpcErrorCode::SrcCurMalformed);
    };
    let mut currency = Currency::zero();
    if !to_currency(&mut currency, code) || currency == bad_currency() || currency == no_currency()
    {
        return Err(RpcErrorCode::SrcCurMalformed);
    }

    let issuer = match object.get("issuer") {
        None if is_xrp_currency(currency) => xrp_account(),
        None => source,
        Some(JsonValue::String(text)) => {
            let parsed = parse_base58_account_id(text).ok_or(RpcErrorCode::SrcIsrMalformed)?;
            if is_xrp_currency(currency) && parsed != xrp_account() {
                return Err(RpcErrorCode::SrcCurMalformed);
            }
            parsed
        }
        Some(_) => return Err(RpcErrorCode::SrcIsrMalformed),
    };
    let asset = Asset::Issue(Issue::new(currency, issuer));
    if !valid_path_asset(asset) {
        return Err(RpcErrorCode::SrcCurMalformed);
    }
    Ok(asset)
}

pub fn parse_path_finder_request(params: &JsonValue) -> Result<PathFinderRequest, Status> {
    let JsonValue::Object(object) = params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    let (source_account, source_account_id) = match object.get("source_account") {
        Some(JsonValue::String(value)) => match parse_base58_account_id(value) {
            Some(account) => (value.clone(), account),
            None => return Err(Status::new(RpcErrorCode::SrcActMalformed)),
        },
        Some(_) => return Err(Status::new(RpcErrorCode::SrcActMalformed)),
        None => return Err(Status::new(RpcErrorCode::SrcActMissing)),
    };

    let (destination_account, destination_account_id) = match object.get("destination_account") {
        Some(JsonValue::String(value)) => match parse_base58_account_id(value) {
            Some(account) => (value.clone(), account),
            None => return Err(Status::new(RpcErrorCode::DstActMalformed)),
        },
        Some(_) => return Err(Status::new(RpcErrorCode::DstActMalformed)),
        None => return Err(Status::new(RpcErrorCode::DstActMissing)),
    };

    let Some(destination_amount) = object.get("destination_amount") else {
        return Err(Status::new(RpcErrorCode::DstAmtMissing));
    };
    let Some(parsed_destination_amount) = parse_amount(destination_amount) else {
        return Err(Status::new(RpcErrorCode::DstAmtMalformed));
    };
    let convert_all = is_convert_all(&parsed_destination_amount);
    if !valid_path_asset(parsed_destination_amount.asset())
        || (!convert_all && parsed_destination_amount.signum() <= 0)
    {
        return Err(Status::new(RpcErrorCode::DstAmtMalformed));
    }

    let send_max = object.get("send_max").cloned();
    if send_max.is_some() && !convert_all {
        return Err(Status::new(RpcErrorCode::DstAmtMalformed));
    }
    let parsed_send_max = send_max
        .as_ref()
        .map(|value| parse_amount(value).ok_or_else(|| Status::new(RpcErrorCode::SendmaxMalformed)))
        .transpose()?;
    if let Some(amount) = parsed_send_max.as_ref()
        && (!valid_path_asset(amount.asset()) || (amount.signum() <= 0 && !is_convert_all(amount)))
    {
        return Err(Status::new(RpcErrorCode::SendmaxMalformed));
    }

    let mut source_assets = match object.get("source_currencies") {
        None => Vec::new(),
        Some(JsonValue::Array(values)) if !values.is_empty() && values.len() <= 18 => values
            .iter()
            .map(|value| parse_source_asset(value, source_account_id).map_err(Status::new))
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(Status::new(RpcErrorCode::SrcCurMalformed)),
    };
    source_assets.sort();
    source_assets.dedup();
    if let Some(send_max) = parsed_send_max.as_ref() {
        let mut constrained = Vec::new();
        for source_asset in source_assets {
            if source_asset.token() != send_max.asset().token() {
                continue;
            }
            match (source_asset, send_max.asset()) {
                (Asset::Issue(source_issue), Asset::Issue(send_issue)) => {
                    if source_issue.account != source_account_id
                        && send_issue.account != source_account_id
                        && source_issue.account != send_issue.account
                    {
                        return Err(Status::new(RpcErrorCode::SrcIsrMalformed));
                    }
                    let issuer = if source_issue.account != source_account_id {
                        source_issue.account
                    } else if send_issue.account != source_account_id {
                        send_issue.account
                    } else {
                        source_account_id
                    };
                    constrained.push(Asset::Issue(Issue::new(source_issue.currency, issuer)));
                }
                (Asset::MPTIssue(_), Asset::MPTIssue(issue)) => {
                    constrained.push(Asset::MPTIssue(issue));
                }
                _ => {}
            }
        }
        source_assets = constrained;
        source_assets.sort();
        source_assets.dedup();
    }

    let domain = match object.get("domain") {
        None => None,
        Some(JsonValue::String(value)) => {
            Some(Uint256::from_hex(value).map_err(|_| Status::new(RpcErrorCode::DomainMalformed))?)
        }
        Some(_) => return Err(Status::new(RpcErrorCode::DomainMalformed)),
    };

    Ok(PathFinderRequest {
        source_account,
        destination_account,
        destination_amount: destination_amount.clone(),
        send_max,
        source_account_id,
        destination_account_id,
        parsed_destination_amount,
        parsed_send_max,
        source_assets,
        domain,
        convert_all,
        id: object.get("id").cloned(),
    })
}

pub fn make_path_find_status(
    _request_id: u64,
    request: &PathFinderRequest,
    result: PathFinderResult,
    full_reply: bool,
    is_legacy: bool,
) -> JsonValue {
    let mut response = BTreeMap::from([
        (
            "source_account".to_owned(),
            JsonValue::String(request.source_account.clone()),
        ),
        (
            "destination_account".to_owned(),
            JsonValue::String(request.destination_account.clone()),
        ),
        (
            "destination_amount".to_owned(),
            request.destination_amount.clone(),
        ),
        ("full_reply".to_owned(), JsonValue::Bool(full_reply)),
        ("alternatives".to_owned(), result.alternatives),
    ]);
    if let Some(id) = request.id.as_ref() {
        response.insert("id".to_owned(), id.clone());
    }
    if let Some(hash) = result.ledger_hash {
        response.insert("ledger_hash".to_owned(), JsonValue::String(hash));
    }
    if let Some(index) = result.ledger_index {
        response.insert(
            "ledger_index".to_owned(),
            JsonValue::Unsigned(u64::from(index)),
        );
    }
    if let Some(validated) = result.validated {
        response.insert("validated".to_owned(), JsonValue::Bool(validated));
    }
    if is_legacy {
        response.insert(
            "destination_currencies".to_owned(),
            JsonValue::Array(
                result
                    .destination_currencies
                    .into_iter()
                    .map(JsonValue::String)
                    .collect(),
            ),
        );
    }
    JsonValue::Object(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(fill: u8) -> String {
        protocol::to_base58(AccountID::from_array([fill; 20]))
    }

    fn base_request() -> BTreeMap<String, JsonValue> {
        BTreeMap::from([
            ("source_account".to_owned(), JsonValue::String(account(1))),
            (
                "destination_account".to_owned(),
                JsonValue::String(account(2)),
            ),
            (
                "destination_amount".to_owned(),
                JsonValue::String("1000000".to_owned()),
            ),
        ])
    }

    fn error(mut request: BTreeMap<String, JsonValue>) -> RpcErrorCode {
        parse_path_finder_request(&JsonValue::Object(std::mem::take(&mut request)))
            .expect_err("request must fail")
            .error_code()
            .expect("error code")
    }

    #[test]
    fn parser_uses_rippled_specific_missing_and_malformed_codes() {
        let mut request = base_request();
        request.remove("source_account");
        assert_eq!(error(request), RpcErrorCode::SrcActMissing);

        let mut request = base_request();
        request.insert(
            "source_account".to_owned(),
            JsonValue::String("bad".to_owned()),
        );
        assert_eq!(error(request), RpcErrorCode::SrcActMalformed);

        let mut request = base_request();
        request.remove("destination_account");
        assert_eq!(error(request), RpcErrorCode::DstActMissing);

        let mut request = base_request();
        request.remove("destination_amount");
        assert_eq!(error(request), RpcErrorCode::DstAmtMissing);
    }

    #[test]
    fn parser_enforces_convert_all_send_max_contract() {
        let mut request = base_request();
        request.insert("send_max".to_owned(), JsonValue::String("10".to_owned()));
        assert_eq!(error(request), RpcErrorCode::DstAmtMalformed);

        let mut request = base_request();
        request.insert(
            "destination_amount".to_owned(),
            JsonValue::String("-1".to_owned()),
        );
        request.insert(
            "send_max".to_owned(),
            JsonValue::String("broken".to_owned()),
        );
        assert_eq!(error(request), RpcErrorCode::SendmaxMalformed);
    }

    #[test]
    fn parser_validates_source_assets_and_domain_and_echoes_client_id() {
        let mut request = base_request();
        request.insert(
            "source_currencies".to_owned(),
            JsonValue::Array(vec![JsonValue::Object(BTreeMap::from([
                ("currency".to_owned(), JsonValue::String("USD".to_owned())),
                ("issuer".to_owned(), JsonValue::String(account(3))),
            ]))]),
        );
        request.insert("domain".to_owned(), JsonValue::String("00".repeat(32)));
        request.insert("id".to_owned(), JsonValue::String("client-id".to_owned()));
        let parsed = parse_path_finder_request(&JsonValue::Object(request)).expect("valid request");
        assert_eq!(parsed.source_assets.len(), 1);
        assert!(parsed.domain.is_some());
        assert_eq!(parsed.id, Some(JsonValue::String("client-id".to_owned())));

        let mut request = base_request();
        request.insert("domain".to_owned(), JsonValue::String("xyz".to_owned()));
        assert_eq!(error(request), RpcErrorCode::DomainMalformed);
    }

    #[test]
    fn parser_deduplicates_source_assets_like_rippleds_asset_set() {
        let source_asset = JsonValue::Object(BTreeMap::from([(
            "currency".to_owned(),
            JsonValue::String("USD".to_owned()),
        )]));
        let mut request = base_request();
        request.insert(
            "source_currencies".to_owned(),
            JsonValue::Array(vec![source_asset.clone(), source_asset]),
        );
        let parsed = parse_path_finder_request(&JsonValue::Object(request)).expect("valid request");
        assert_eq!(parsed.source_assets.len(), 1);

        let mut request = base_request();
        request.insert(
            "source_currencies".to_owned(),
            JsonValue::Array(vec![JsonValue::Object(BTreeMap::from([
                ("currency".to_owned(), JsonValue::String("XRP".to_owned())),
                ("issuer".to_owned(), JsonValue::String(account(3))),
            ]))]),
        );
        assert_eq!(error(request), RpcErrorCode::SrcCurMalformed);

        let mut request = base_request();
        request.insert(
            "source_currencies".to_owned(),
            JsonValue::Array(vec![JsonValue::Object(BTreeMap::from([
                ("currency".to_owned(), JsonValue::String("USD".to_owned())),
                ("issuer".to_owned(), JsonValue::Null),
            ]))]),
        );
        assert_eq!(error(request), RpcErrorCode::SrcIsrMalformed);
    }
}
