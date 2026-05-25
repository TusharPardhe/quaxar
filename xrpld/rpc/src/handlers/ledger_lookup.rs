//! Narrow RPC ledger lookup helpers ported from `xrpld/rpc/detail/RPCLedgerHelpers.*`.

#![allow(dead_code)]

use std::{collections::BTreeMap, time::Duration};

use basics::base_uint::Uint256;
use protocol::JsonValue;

pub use crate::state::role::{Role as RpcRole, is_unlimited};
use crate::state::tuning::Tuning;
pub use crate::status::{RpcErrorCode, Status as RpcStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerLookupLedger {
    pub hash: Uint256,
    pub seq: u32,
    pub open: bool,
}

pub trait LedgerLookupSource {
    fn get_ledger_by_hash(&self, hash: Uint256) -> Option<LedgerLookupLedger>;
    fn get_ledger_by_seq(&self, seq: u32) -> Option<LedgerLookupLedger>;
    fn get_current_ledger(&self) -> Option<LedgerLookupLedger>;
    fn get_closed_ledger(&self) -> Option<LedgerLookupLedger>;
    fn get_validated_ledger(&self) -> Option<LedgerLookupLedger>;
    fn get_valid_ledger_index(&self) -> u32;
    fn get_validated_ledger_age(&self) -> Duration;
    fn is_validated(&self, ledger: &LedgerLookupLedger) -> bool;
    fn standalone(&self) -> bool {
        false
    }
}

pub struct LedgerLookupContext<'a, S> {
    pub params: &'a JsonValue,
    pub source: &'a S,
    pub api_version: u32,
    pub role: RpcRole,
}

enum LedgerIndexRequest {
    Current,
    Closed,
    Validated,
    Sequence(u32),
}

enum LedgerRequest {
    Hash(Uint256),
    Index(LedgerIndexRequest),
}

fn is_validated_old<S: LedgerLookupSource>(source: &S) -> bool {
    if source.standalone() {
        return false;
    }
    source.get_validated_ledger_age() > Tuning::MAX_VALIDATED_LEDGER_AGE
}

fn ledger_by_shortcut<S: LedgerLookupSource>(
    source: &S,
    request: LedgerIndexRequest,
    api_version: u32,
) -> Result<LedgerLookupLedger, RpcStatus> {
    if is_validated_old(source) {
        return Err(if api_version == 1 {
            RpcStatus::new(RpcErrorCode::NoNetwork)
        } else {
            RpcStatus::new(RpcErrorCode::NotSynced)
        });
    }

    match request {
        LedgerIndexRequest::Validated => source.get_validated_ledger().ok_or_else(|| {
            if api_version == 1 {
                RpcStatus::new(RpcErrorCode::NoNetwork)
            } else {
                RpcStatus::new(RpcErrorCode::NotSynced)
            }
        }),
        LedgerIndexRequest::Current => {
            let ledger = source.get_current_ledger().ok_or_else(|| {
                if api_version == 1 {
                    RpcStatus::new(RpcErrorCode::NoNetwork)
                } else {
                    RpcStatus::new(RpcErrorCode::NotSynced)
                }
            })?;

            if ledger.seq + 10 < source.get_valid_ledger_index() {
                return Err(if api_version == 1 {
                    RpcStatus::new(RpcErrorCode::NoNetwork)
                } else {
                    RpcStatus::new(RpcErrorCode::NotSynced)
                });
            }

            Ok(ledger)
        }
        LedgerIndexRequest::Closed => {
            let ledger = source.get_closed_ledger().ok_or_else(|| {
                if api_version == 1 {
                    RpcStatus::new(RpcErrorCode::NoNetwork)
                } else {
                    RpcStatus::new(RpcErrorCode::NotSynced)
                }
            })?;

            if ledger.seq + 10 < source.get_valid_ledger_index() {
                return Err(if api_version == 1 {
                    RpcStatus::new(RpcErrorCode::NoNetwork)
                } else {
                    RpcStatus::new(RpcErrorCode::NotSynced)
                });
            }

            Ok(ledger)
        }
        LedgerIndexRequest::Sequence(seq) => source.get_ledger_by_seq(seq).ok_or_else(|| {
            if api_version == 1 {
                RpcStatus::new(RpcErrorCode::NoNetwork)
            } else {
                RpcStatus::new(RpcErrorCode::NotSynced)
            }
        }),
    }
}

fn parse_index_request(value: &JsonValue) -> Result<LedgerIndexRequest, RpcStatus> {
    match value {
        JsonValue::String(text) if text == "current" || text.is_empty() => {
            Ok(LedgerIndexRequest::Current)
        }
        JsonValue::String(text) if text == "validated" => Ok(LedgerIndexRequest::Validated),
        JsonValue::String(text) if text == "closed" => Ok(LedgerIndexRequest::Closed),
        JsonValue::String(text) => text
            .parse::<u32>()
            .map(LedgerIndexRequest::Sequence)
            .map_err(|_| RpcStatus::expected_field_error("ledger_index", "string or number")),
        JsonValue::Unsigned(value) => u32::try_from(*value)
            .map(LedgerIndexRequest::Sequence)
            .map_err(|_| RpcStatus::expected_field_error("ledger_index", "string or number")),
        JsonValue::Signed(value) if *value >= 0 => u32::try_from(*value as u64)
            .map(LedgerIndexRequest::Sequence)
            .map_err(|_| RpcStatus::expected_field_error("ledger_index", "string or number")),
        _ => Err(RpcStatus::expected_field_error(
            "ledger_index",
            "string or number",
        )),
    }
}

fn parse_legacy_index_request(value: &JsonValue) -> Result<LedgerIndexRequest, RpcStatus> {
    match value {
        JsonValue::String(text) if text == "current" || text.is_empty() => {
            Ok(LedgerIndexRequest::Current)
        }
        JsonValue::String(text) if text == "validated" => Ok(LedgerIndexRequest::Validated),
        JsonValue::String(text) if text == "closed" => Ok(LedgerIndexRequest::Closed),
        JsonValue::String(text) => text
            .parse::<u32>()
            .map(LedgerIndexRequest::Sequence)
            .map_err(|_| RpcStatus::expected_field_error("ledger", "string or number")),
        JsonValue::Unsigned(value) => u32::try_from(*value)
            .map(LedgerIndexRequest::Sequence)
            .map_err(|_| RpcStatus::expected_field_error("ledger", "string or number")),
        JsonValue::Signed(value) if *value >= 0 => u32::try_from(*value as u64)
            .map(LedgerIndexRequest::Sequence)
            .map_err(|_| RpcStatus::expected_field_error("ledger", "string or number")),
        _ => Err(RpcStatus::expected_field_error(
            "ledger",
            "string or number",
        )),
    }
}

fn parse_legacy_request(value: &JsonValue) -> Result<LedgerRequest, RpcStatus> {
    if let JsonValue::String(text) = value
        && text.len() == 64
        && let Ok(hash) = Uint256::from_hex(text)
    {
        return Ok(LedgerRequest::Hash(hash));
    }

    parse_legacy_index_request(value).map(LedgerRequest::Index)
}

fn resolve_ledger<S: LedgerLookupSource>(
    context: &LedgerLookupContext<'_, S>,
) -> Result<LedgerLookupLedger, RpcStatus> {
    let params = context.params;
    let has_ledger = matches!(params, JsonValue::Object(object) if object.contains_key("ledger"));
    let has_hash =
        matches!(params, JsonValue::Object(object) if object.contains_key("ledger_hash"));
    let has_index =
        matches!(params, JsonValue::Object(object) if object.contains_key("ledger_index"));

    if (has_ledger as u8 + has_hash as u8 + has_index as u8) > 1 {
        if has_ledger {
            return Err(RpcStatus::make_param_error(
                "Exactly one of 'ledger', 'ledger_hash', or 'ledger_index' can be specified.",
            ));
        }
        return Err(RpcStatus::make_param_error(
            "Exactly one of 'ledger_hash' or 'ledger_index' can be specified.",
        ));
    }

    let JsonValue::Object(object) = params else {
        return Err(RpcStatus::make_param_error(
            "Exactly one of 'ledger_hash' or 'ledger_index' can be specified.",
        ));
    };

    if let Some(legacy) = object.get("ledger") {
        return match parse_legacy_request(legacy)? {
            LedgerRequest::Hash(hash) => context
                .source
                .get_ledger_by_hash(hash)
                .ok_or_else(|| RpcStatus::new(RpcErrorCode::LedgerNotFound)),
            LedgerRequest::Index(LedgerIndexRequest::Sequence(seq)) => {
                let mut ledger = context.source.get_ledger_by_seq(seq);
                if ledger.is_none() {
                    ledger = context
                        .source
                        .get_current_ledger()
                        .filter(|current| current.seq == seq);
                }

                let ledger = ledger.ok_or_else(|| RpcStatus::new(RpcErrorCode::LedgerNotFound))?;
                if ledger.seq > context.source.get_valid_ledger_index()
                    && is_validated_old(context.source)
                {
                    return Err(if context.api_version == 1 {
                        RpcStatus::new(RpcErrorCode::NoNetwork)
                    } else {
                        RpcStatus::new(RpcErrorCode::NotSynced)
                    });
                }
                Ok(ledger)
            }
            LedgerRequest::Index(shortcut) => {
                ledger_by_shortcut(context.source, shortcut, context.api_version)
            }
        };
    }

    if let Some(hash) = object.get("ledger_hash") {
        let JsonValue::String(hash) = hash else {
            return Err(RpcStatus::expected_field_error("ledger_hash", "hex string"));
        };
        let hash = Uint256::from_hex(hash)
            .map_err(|_| RpcStatus::expected_field_error("ledger_hash", "hex string"))?;
        return context
            .source
            .get_ledger_by_hash(hash)
            .ok_or_else(|| RpcStatus::new(RpcErrorCode::LedgerNotFound));
    }

    let request = if let Some(index) = object.get("ledger_index") {
        parse_index_request(index)?
    } else {
        LedgerIndexRequest::Current
    };

    match request {
        LedgerIndexRequest::Sequence(seq) => {
            let mut ledger = context.source.get_ledger_by_seq(seq);
            if ledger.is_none() {
                ledger = context
                    .source
                    .get_current_ledger()
                    .filter(|current| current.seq == seq);
            }

            let ledger = ledger.ok_or_else(|| RpcStatus::new(RpcErrorCode::LedgerNotFound))?;
            if ledger.seq > context.source.get_valid_ledger_index()
                && is_validated_old(context.source)
            {
                return Err(if context.api_version == 1 {
                    RpcStatus::new(RpcErrorCode::NoNetwork)
                } else {
                    RpcStatus::new(RpcErrorCode::NotSynced)
                });
            }
            Ok(ledger)
        }
        shortcut => ledger_by_shortcut(context.source, shortcut, context.api_version),
    }
}

pub fn lookup_ledger_with_result<S: LedgerLookupSource>(
    context: &LedgerLookupContext<'_, S>,
) -> Result<(LedgerLookupLedger, JsonValue), RpcStatus> {
    let ledger = resolve_ledger(context)?;
    let mut result = JsonValue::Object(BTreeMap::new());
    if let JsonValue::Object(object) = &mut result {
        if !ledger.open {
            object.insert(
                "ledger_hash".to_owned(),
                JsonValue::String(ledger.hash.to_string()),
            );
            object.insert(
                "ledger_index".to_owned(),
                JsonValue::Unsigned(u64::from(ledger.seq)),
            );
        } else {
            object.insert(
                "ledger_current_index".to_owned(),
                JsonValue::Unsigned(u64::from(ledger.seq)),
            );
        }
        object.insert(
            "validated".to_owned(),
            JsonValue::Bool(context.source.is_validated(&ledger)),
        );
    }
    Ok((ledger, result))
}

pub fn lookup_ledger<S: LedgerLookupSource>(
    context: &LedgerLookupContext<'_, S>,
) -> Result<JsonValue, RpcStatus> {
    lookup_ledger_with_result(context).map(|(_, result)| result)
}

pub fn lookup_ledger_json<S: LedgerLookupSource>(
    context: &LedgerLookupContext<'_, S>,
) -> JsonValue {
    let mut result = JsonValue::Object(BTreeMap::new());
    match lookup_ledger(context) {
        Ok(value) => value,
        Err(status) => {
            status.inject(&mut result);
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcErrorCode, RpcRole,
        RpcStatus, is_unlimited, lookup_ledger_json,
    };
    use basics::base_uint::Uint256;
    use protocol::JsonValue;
    use std::{collections::BTreeMap, time::Duration};

    #[derive(Debug)]
    struct Source {
        current: Option<LedgerLookupLedger>,
        closed: Option<LedgerLookupLedger>,
        validated: Option<LedgerLookupLedger>,
        by_seq: BTreeMap<u32, LedgerLookupLedger>,
        by_hash: BTreeMap<String, LedgerLookupLedger>,
        valid_index: u32,
        validated_age: Duration,
        standalone: bool,
    }

    impl LedgerLookupSource for Source {
        fn get_ledger_by_hash(&self, hash: Uint256) -> Option<LedgerLookupLedger> {
            self.by_hash.get(&hash.to_string()).copied()
        }

        fn get_ledger_by_seq(&self, seq: u32) -> Option<LedgerLookupLedger> {
            self.by_seq.get(&seq).copied()
        }

        fn get_current_ledger(&self) -> Option<LedgerLookupLedger> {
            self.current
        }

        fn get_closed_ledger(&self) -> Option<LedgerLookupLedger> {
            self.closed
        }

        fn get_validated_ledger(&self) -> Option<LedgerLookupLedger> {
            self.validated
        }

        fn get_valid_ledger_index(&self) -> u32 {
            self.valid_index
        }

        fn get_validated_ledger_age(&self) -> Duration {
            self.validated_age
        }

        fn is_validated(&self, ledger: &LedgerLookupLedger) -> bool {
            self.validated
                .map(|valid| valid.seq == ledger.seq)
                .unwrap_or(false)
        }

        fn standalone(&self) -> bool {
            self.standalone
        }
    }

    fn ledger(seq: u32, fill: u8, open: bool) -> LedgerLookupLedger {
        LedgerLookupLedger {
            hash: Uint256::from_array([fill; 32]),
            seq,
            open,
        }
    }

    fn context<'a>(params: &'a JsonValue, source: &'a Source) -> LedgerLookupContext<'a, Source> {
        LedgerLookupContext {
            params,
            source,
            api_version: 2,
            role: RpcRole::User,
        }
    }

    #[test]
    fn lookup_ledger_returns_current_shape() {
        let source = Source {
            current: Some(ledger(10, 0xAA, true)),
            closed: Some(ledger(9, 0xAB, false)),
            validated: Some(ledger(8, 0xAC, false)),
            by_seq: BTreeMap::new(),
            by_hash: BTreeMap::new(),
            valid_index: 10,
            validated_age: Duration::from_secs(10),
            standalone: false,
        };

        let params = JsonValue::Object(BTreeMap::new());
        let json = lookup_ledger_json(&context(&params, &source));
        let JsonValue::Object(object) = json else {
            panic!("json should be an object");
        };
        assert_eq!(
            object.get("ledger_current_index"),
            Some(&JsonValue::Unsigned(10))
        );
        assert_eq!(object.get("validated"), Some(&JsonValue::Bool(false)));
    }

    #[test]
    fn lookup_ledger_rejects_multiple_selector_fields() {
        let source = Source {
            current: Some(ledger(10, 0xAA, true)),
            closed: None,
            validated: None,
            by_seq: BTreeMap::new(),
            by_hash: BTreeMap::new(),
            valid_index: 10,
            validated_age: Duration::from_secs(10),
            standalone: false,
        };

        let params = JsonValue::Object(BTreeMap::from([
            ("ledger".to_owned(), JsonValue::String("current".to_owned())),
            (
                "ledger_index".to_owned(),
                JsonValue::String("current".to_owned()),
            ),
        ]));

        let json = lookup_ledger_json(&context(&params, &source));
        let JsonValue::Object(object) = json else {
            panic!("json should be an object");
        };
        assert_eq!(
            object.get("error"),
            Some(&JsonValue::String("invalidParams".to_owned()))
        );
    }

    #[test]
    fn rpc_status_helpers_match_cxx_error_shape() {
        let status =
            RpcStatus::with_message(RpcErrorCode::InvalidParams, "Invalid field 'ledger'.");
        let mut json = JsonValue::Object(BTreeMap::new());
        status.inject(&mut json);
        let JsonValue::Object(object) = json else {
            panic!("json should be an object");
        };
        assert_eq!(object.get("error_code"), Some(&JsonValue::Signed(31)));
        assert_eq!(
            object.get("error_message"),
            Some(&JsonValue::String("Invalid field 'ledger'.".to_owned()))
        );
    }

    #[test]
    fn role_helper_matches_admin_and_identified_only() {
        assert!(is_unlimited(RpcRole::Admin));
        assert!(is_unlimited(RpcRole::Identified));
        assert!(!is_unlimited(RpcRole::Guest));
    }
}
