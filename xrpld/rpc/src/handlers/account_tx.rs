//! `account_tx` RPC handler aligned with `handlers/account/the reference source`.

use std::collections::BTreeMap;

use app::{TransStatus, Transaction};
use basics::{
    base_uint::Uint256,
    chrono::{NetClockTimePoint, to_string_iso},
    str_hex::str_hex,
};
use protocol::{AccountID, JsonOptions, JsonValue, parse_base58_account_id};

use crate::commands::rpc_helpers::read_limit_field;
use crate::handlers::delivered_amount::insert_delivered_amount;
use crate::handlers::ledger_lookup::{LedgerLookupLedger, LedgerLookupSource};
use crate::handlers::mp_token_issuance_id::insert_mp_token_issuance_id;
use crate::insert_deliver_max;
use crate::state::role::Role;
use crate::state::tuning::Tuning;
use crate::state::tx_support::TxRecord;
use crate::status::{RpcErrorCode, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountTxMarker {
    pub ledger: u32,
    pub seq: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountTxLedgerRange {
    pub min: u32,
    pub max: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountTxLedgerSpecifier {
    Hash(Uint256),
    Sequence(u32),
    Current,
    Closed,
    Validated,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountTxQuery {
    pub account: AccountID,
    pub ledger: Option<AccountTxLedgerSpecifier>,
    pub ledger_range: AccountTxLedgerRange,
    pub binary: bool,
    pub forward: bool,
    pub limit: u32,
    pub marker: Option<AccountTxMarker>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountTxPage {
    pub ledger_range: AccountTxLedgerRange,
    pub limit: u32,
    pub marker: Option<AccountTxMarker>,
    pub transactions: Vec<TxRecord>,
}

pub trait AccountTxSource: LedgerLookupSource {
    fn validated_range(&self) -> Option<AccountTxLedgerRange>;
    fn page(&self, query: &AccountTxQuery) -> Result<AccountTxPage, Status>;
    fn get_hash_by_seq(&self, seq: u32) -> Option<Uint256> {
        self.get_ledger_by_seq(seq).map(|ledger| ledger.hash)
    }
    fn get_close_time_by_seq(&self, _seq: u32) -> Option<NetClockTimePoint> {
        None
    }
}

fn ensure_object(json: &mut JsonValue) -> &mut BTreeMap<String, JsonValue> {
    if !matches!(json, JsonValue::Object(_)) {
        *json = JsonValue::Object(BTreeMap::new());
    }
    let JsonValue::Object(object) = json else {
        unreachable!("json shell should be an object");
    };
    object
}

fn parse_bool_field(params: &JsonValue, field: &str, api_version: u32) -> Result<bool, Status> {
    let JsonValue::Object(object) = params else {
        return Ok(false);
    };
    let Some(value) = object.get(field) else {
        return Ok(false);
    };
    match value {
        JsonValue::Bool(value) => Ok(*value),
        _ if api_version > 1 => Err(Status::invalid_field_error(field)),
        JsonValue::Unsigned(value) => Ok(*value != 0),
        JsonValue::Signed(value) => Ok(*value != 0),
        JsonValue::String(value) => Ok(!value.is_empty()),
        _ => Ok(false),
    }
}

fn parse_ledger_specifier(
    params: &JsonValue,
    api_version: u32,
) -> Result<Option<AccountTxLedgerSpecifier>, Status> {
    let JsonValue::Object(object) = params else {
        return Ok(None);
    };

    let has_range =
        object.contains_key("ledger_index_min") || object.contains_key("ledger_index_max");
    if api_version > 1
        && has_range
        && (object.contains_key("ledger_hash") || object.contains_key("ledger_index"))
    {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    }

    if let Some(JsonValue::String(hash)) = object.get("ledger_hash") {
        let hash = Uint256::from_hex(hash)
            .map_err(|_| Status::expected_field_error("ledger_hash", "hex string"))?;
        return Ok(Some(AccountTxLedgerSpecifier::Hash(hash)));
    }

    let Some(value) = object.get("ledger_index") else {
        return Ok(None);
    };

    let specifier = match value {
        JsonValue::Unsigned(value) => AccountTxLedgerSpecifier::Sequence(
            u32::try_from(*value).map_err(|_| Status::invalid_field_error("ledger_index"))?,
        ),
        JsonValue::Signed(value) if *value >= 0 => AccountTxLedgerSpecifier::Sequence(
            u32::try_from(*value as u64)
                .map_err(|_| Status::invalid_field_error("ledger_index"))?,
        ),
        JsonValue::String(value) if value == "current" || value.is_empty() => {
            AccountTxLedgerSpecifier::Current
        }
        JsonValue::String(value) if value == "closed" => AccountTxLedgerSpecifier::Closed,
        JsonValue::String(value) if value == "validated" => AccountTxLedgerSpecifier::Validated,
        _ => return Err(Status::invalid_field_error("ledger_index")),
    };

    Ok(Some(specifier))
}

fn resolve_specifier<S: AccountTxSource>(
    source: &S,
    specifier: AccountTxLedgerSpecifier,
) -> Option<LedgerLookupLedger> {
    match specifier {
        AccountTxLedgerSpecifier::Hash(hash) => source.get_ledger_by_hash(hash),
        AccountTxLedgerSpecifier::Sequence(seq) => source.get_ledger_by_seq(seq).or_else(|| {
            source
                .get_current_ledger()
                .filter(|ledger| ledger.seq == seq)
        }),
        AccountTxLedgerSpecifier::Current => source.get_current_ledger(),
        AccountTxLedgerSpecifier::Closed => source.get_closed_ledger(),
        AccountTxLedgerSpecifier::Validated => source.get_validated_ledger(),
    }
}

fn ledger_range_from_params<S: AccountTxSource>(
    source: &S,
    params: &JsonValue,
    api_version: u32,
    specifier: Option<AccountTxLedgerSpecifier>,
) -> Result<AccountTxLedgerRange, Status> {
    let Some(validated) = source.validated_range() else {
        return Err(if api_version == 1 {
            Status::new(RpcErrorCode::LedgerIndexesInvalid)
        } else {
            Status::new(RpcErrorCode::NotSynced)
        });
    };

    if let Some(specifier) = specifier {
        let Some(ledger) = resolve_specifier(source, specifier) else {
            return Err(Status::new(RpcErrorCode::LedgerNotFound));
        };
        if !source.is_validated(&ledger) || ledger.seq < validated.min || ledger.seq > validated.max
        {
            return Err(Status::new(RpcErrorCode::LedgerNotValidated));
        }
        return Ok(AccountTxLedgerRange {
            min: ledger.seq,
            max: ledger.seq,
        });
    }

    let JsonValue::Object(object) = params else {
        return Ok(validated);
    };

    let requested_min = match object.get("ledger_index_min") {
        Some(JsonValue::Unsigned(value)) => Some(u32::try_from(*value).unwrap_or(u32::MAX)),
        Some(JsonValue::Signed(value)) if *value >= 0 => {
            Some(u32::try_from(*value as u64).unwrap_or(u32::MAX))
        }
        Some(_) => Some(0),
        None => None,
    };
    let requested_max = match object.get("ledger_index_max") {
        Some(JsonValue::Unsigned(value)) => Some(u32::try_from(*value).unwrap_or(u32::MAX)),
        Some(JsonValue::Signed(value)) if *value >= 0 => {
            Some(u32::try_from(*value as u64).unwrap_or(u32::MAX))
        }
        Some(_) => Some(u32::MAX),
        None => None,
    };

    if api_version > 1
        && (requested_max.is_some_and(|max| max > validated.max && max != u32::MAX)
            || requested_min.is_some_and(|min| min < validated.min && min != 0))
    {
        return Err(Status::new(RpcErrorCode::LedgerIndexMalformed));
    }

    let min = requested_min.unwrap_or(validated.min);
    let max = requested_max.unwrap_or(validated.max);

    let min = min.max(validated.min);
    let max = max.min(validated.max);
    if max < min {
        return Err(if api_version == 1 {
            Status::new(RpcErrorCode::LedgerIndexesInvalid)
        } else {
            Status::new(RpcErrorCode::InvalidLedgerRange)
        });
    }

    Ok(AccountTxLedgerRange { min, max })
}

fn parse_marker(params: &JsonValue) -> Result<Option<AccountTxMarker>, Status> {
    let JsonValue::Object(object) = params else {
        return Ok(None);
    };
    let Some(JsonValue::Object(marker)) = object.get("marker") else {
        return Ok(None);
    };
    let Some(JsonValue::Unsigned(ledger)) = marker.get("ledger") else {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            "invalid marker. Provide ledger index via ledger field, and transaction sequence number via seq field",
        ));
    };
    let Some(JsonValue::Unsigned(seq)) = marker.get("seq") else {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            "invalid marker. Provide ledger index via ledger field, and transaction sequence number via seq field",
        ));
    };
    Ok(Some(AccountTxMarker {
        ledger: u32::try_from(*ledger).map_err(|_| Status::invalid_field_error("marker"))?,
        seq: u32::try_from(*seq).map_err(|_| Status::invalid_field_error("marker"))?,
    }))
}

fn make_owner(record: &TxRecord) -> Transaction {
    let mut owner = Transaction::new(record.txn.clone());
    owner.set_status_with_ledger(
        if record.validated {
            TransStatus::COMMITTED
        } else {
            TransStatus::NEW
        },
        record.ledger_index,
        record.txn_index,
        record.network_id,
    );
    owner
}

fn insert_v1_transaction(target: &mut JsonValue, record: &TxRecord, binary: bool) {
    let txn = record.txn.as_ref();
    let owner = make_owner(record);
    *target = owner.get_json_with_close_time(
        JsonOptions::INCLUDE_DATE,
        binary,
        record
            .close_time
            .map(|close_time| i64::from(close_time.as_seconds())),
    );
    if !binary {
        insert_deliver_max(target, txn.get_txn_type(), 1);
    }
    if let Some(meta) = &record.meta {
        let mut meta_json = meta.get_json(JsonOptions::INCLUDE_DATE);
        insert_delivered_amount(
            &mut meta_json,
            record.ledger_index,
            record.close_time.map(|close_time| close_time.as_seconds()),
            txn,
            meta,
        );
        insert_mp_token_issuance_id(&mut meta_json, txn, meta);
        ensure_object(target).insert("meta".to_owned(), meta_json);
    }
    ensure_object(target).insert("validated".to_owned(), JsonValue::Bool(record.validated));
}

fn insert_v2_transaction(target: &mut JsonValue, record: &TxRecord, binary: bool) {
    let txn = record.txn.as_ref();
    let owner = make_owner(record);
    let object = ensure_object(target);
    if binary {
        object.insert(
            "tx_blob".to_owned(),
            JsonValue::String(str_hex(txn.get_serializer().data())),
        );
        if let Some(meta) = &record.meta {
            object.insert(
                "meta_blob".to_owned(),
                JsonValue::String(str_hex(meta.get_as_object().get_serializer().data())),
            );
        }
    } else {
        let mut tx_json = owner.get_json_with_close_time(
            JsonOptions::INCLUDE_DATE | JsonOptions::DISABLE_API_PRIOR_V2,
            false,
            record
                .close_time
                .map(|close_time| i64::from(close_time.as_seconds())),
        );
        insert_deliver_max(&mut tx_json, txn.get_txn_type(), 2);
        object.insert("tx_json".to_owned(), tx_json);
        if let Some(meta) = &record.meta {
            let mut meta_json = meta.get_json(JsonOptions::INCLUDE_DATE);
            insert_delivered_amount(
                &mut meta_json,
                record.ledger_index,
                record.close_time.map(|close_time| close_time.as_seconds()),
                txn,
                meta,
            );
            insert_mp_token_issuance_id(&mut meta_json, txn, meta);
            object.insert("meta".to_owned(), meta_json);
        }
    }

    object.insert(
        "hash".to_owned(),
        JsonValue::String(txn.get_transaction_id().to_string()),
    );
    object.insert(
        "ledger_index".to_owned(),
        JsonValue::Unsigned(u64::from(record.ledger_index)),
    );
    if let Some(ledger_hash) = record.ledger_hash {
        object.insert(
            "ledger_hash".to_owned(),
            JsonValue::String(ledger_hash.to_string()),
        );
    }
    if let Some(close_time) = record.close_time {
        object.insert(
            "close_time_iso".to_owned(),
            JsonValue::String(to_string_iso(close_time)),
        );
    }
    object.insert("validated".to_owned(), JsonValue::Bool(record.validated));
}

pub fn do_account_tx<S: AccountTxSource>(
    params: &JsonValue,
    role: Role,
    api_version: u32,
    source: &S,
) -> JsonValue {
    tracing::trace!(target: "rpc", method = "account_tx", "account_tx query");
    let JsonValue::Object(object) = params else {
        let mut error = JsonValue::Object(BTreeMap::new());
        Status::new(RpcErrorCode::InvalidParams).inject(&mut error);
        return error;
    };

    let Some(JsonValue::String(account)) = object.get("account") else {
        let mut error = JsonValue::Object(BTreeMap::new());
        if object.contains_key("account") {
            Status::invalid_field_error("account").inject(&mut error);
        } else {
            Status::missing_field_error("account").inject(&mut error);
        }
        return error;
    };

    let Some(account_id) = parse_base58_account_id(account) else {
        let mut error = JsonValue::Object(BTreeMap::new());
        Status::new(RpcErrorCode::ActMalformed).inject(&mut error);
        return error;
    };

    let response = (|| -> Result<JsonValue, Status> {
        let binary = parse_bool_field(params, "binary", api_version)?;
        let forward = parse_bool_field(params, "forward", api_version)?;
        let limit = read_limit_field(params, role, Tuning::ACCOUNT_TX)?;
        let specifier = parse_ledger_specifier(params, api_version)?;
        let ledger_range = ledger_range_from_params(source, params, api_version, specifier)?;
        let marker = parse_marker(params)?;
        let query = AccountTxQuery {
            account: account_id,
            ledger: specifier,
            ledger_range,
            binary,
            forward,
            limit,
            marker,
        };
        let page = source.page(&query)?;
        let mut response = JsonValue::Object(BTreeMap::from([
            ("validated".to_owned(), JsonValue::Bool(true)),
            (
                "limit".to_owned(),
                JsonValue::Unsigned(u64::from(page.limit)),
            ),
            ("account".to_owned(), JsonValue::String(account.clone())),
            (
                "ledger_index_min".to_owned(),
                JsonValue::Unsigned(u64::from(page.ledger_range.min)),
            ),
            (
                "ledger_index_max".to_owned(),
                JsonValue::Unsigned(u64::from(page.ledger_range.max)),
            ),
            ("transactions".to_owned(), JsonValue::Array(Vec::new())),
        ]));
        let JsonValue::Array(transactions) = ensure_object(&mut response)
            .get_mut("transactions")
            .expect("transactions should exist")
        else {
            unreachable!("transactions should be an array");
        };
        for record in &page.transactions {
            let mut tx = JsonValue::Object(BTreeMap::new());
            if api_version > 1 {
                insert_v2_transaction(&mut tx, record, binary);
            } else {
                insert_v1_transaction(&mut tx, record, binary);
            }
            if api_version > 1
                && binary
                && let Some(ledger_hash) = record
                    .ledger_hash
                    .or_else(|| source.get_hash_by_seq(record.ledger_index))
            {
                ensure_object(&mut tx).insert(
                    "ledger_hash".to_owned(),
                    JsonValue::String(ledger_hash.to_string()),
                );
            }
            if api_version > 1
                && let Some(close_time) = record
                    .close_time
                    .or_else(|| source.get_close_time_by_seq(record.ledger_index))
            {
                ensure_object(&mut tx).insert(
                    "close_time_iso".to_owned(),
                    JsonValue::String(to_string_iso(close_time)),
                );
            }
            transactions.push(tx);
        }
        if let Some(marker) = page.marker {
            ensure_object(&mut response).insert(
                "marker".to_owned(),
                JsonValue::Object(BTreeMap::from([
                    (
                        "ledger".to_owned(),
                        JsonValue::Unsigned(u64::from(marker.ledger)),
                    ),
                    ("seq".to_owned(), JsonValue::Unsigned(u64::from(marker.seq))),
                ])),
            );
        }
        Ok(response)
    })();

    match response {
        Ok(response) => response,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            error
        }
    }
}
