//! `server_definitions` RPC slice.
//!
//! This is a direct adapter over the protocol-owned registries that already
//! exist in Rust. It mirrors the reference catalog dump and the optional `hash`
//! short-circuit without inventing any new registry layer.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    sync::{Arc, OnceLock},
};

use basics::base_uint::Uint256;
use protocol::{
    InnerObjectFormats, JsonValue, LedgerFormats, SERIALIZED_TYPE_NAME_MAP, SerializedTypeId,
    TxFormats, all_sfields, getAllLedgerFlags, getAllTxFlags, getAsfFlagMap, sha512_half,
    trans_results,
};

use crate::commands::rpc_helpers::invalid_field_error;

// Pre-serialized JSON bytes for the full server_definitions response (no hash
// filter). Built once on first request via `OnceLock`; subsequent calls return
// the same `Arc` with zero allocation.
//
// Uses `json_to_compact_string` (defined below in this file) so there is no
// external-crate dependency required for serialisation.
static SERVER_DEFS_BYTES: OnceLock<std::sync::Arc<str>> = OnceLock::new();

fn server_defs_compact_str() -> &'static std::sync::Arc<str> {
    SERVER_DEFS_BYTES.get_or_init(|| {
        // server_definitions() is itself OnceLock-cached, so this only
        // computes the JsonValue tree once.  We then serialise it once here.
        json_to_compact_string(server_definitions().get()).into()
    })
}

/// Returns the full `server_definitions` response pre-serialised as a compact
/// JSON `Arc<str>`.  Callers in the HTTP/WS transport can return this directly
/// as a response body with no further allocations.
///
/// Returns `None` when the caller supplied a `hash` parameter that matches the
/// current definitions hash (short-circuit "not changed" path).
pub fn do_server_definitions_cached(params: &JsonValue) -> Option<std::sync::Arc<str>> {
    let requested_hash = match params {
        JsonValue::Object(object) => match object.get("hash") {
            Some(JsonValue::String(hash)) => {
                let Ok(hash) = Uint256::from_hex(hash) else {
                    // Invalid hash param — let do_server_definitions handle the error.
                    return None;
                };
                Some(hash)
            }
            Some(_) => return None, // invalid type — fall through to error path
            None => None,
        },
        _ => None,
    };

    let defs = server_definitions();
    if requested_hash.is_some_and(|hash| defs.hash_matches(hash)) {
        // Client already has current definitions; skip the full response.
        return None;
    }

    Some(Arc::clone(server_defs_compact_str()))
}

struct ServerDefinitions {
    defs_hash: Uint256,
    defs: JsonValue,
}

impl ServerDefinitions {
    fn new() -> Self {
        let mut defs = BTreeMap::new();
        defs.insert("TYPES".to_owned(), build_types());
        defs.insert("LEDGER_ENTRY_TYPES".to_owned(), build_ledger_entry_types());
        defs.insert("FIELDS".to_owned(), build_fields());
        defs.insert(
            "TRANSACTION_RESULTS".to_owned(),
            build_transaction_results(),
        );
        defs.insert("TRANSACTION_TYPES".to_owned(), build_transaction_types());
        defs.insert(
            "TRANSACTION_FORMATS".to_owned(),
            build_transaction_formats(),
        );
        defs.insert(
            "LEDGER_ENTRY_FORMATS".to_owned(),
            build_ledger_entry_formats(),
        );
        defs.insert(
            "INNER_OBJECT_FORMATS".to_owned(),
            build_inner_object_formats(),
        );
        defs.insert("TRANSACTION_FLAGS".to_owned(), build_transaction_flags());
        defs.insert("LEDGER_ENTRY_FLAGS".to_owned(), build_ledger_entry_flags());
        defs.insert("ACCOUNT_SET_FLAGS".to_owned(), build_account_set_flags());

        let defs_without_hash = JsonValue::Object(defs.clone());
        let defs_hash = sha512_half(json_to_compact_string(&defs_without_hash).as_bytes());
        defs.insert("hash".to_owned(), JsonValue::String(defs_hash.to_string()));

        Self {
            defs_hash,
            defs: JsonValue::Object(defs),
        }
    }

    fn hash_matches(&self, hash: Uint256) -> bool {
        self.defs_hash == hash
    }

    fn get(&self) -> &JsonValue {
        &self.defs
    }
}

fn server_definitions() -> &'static ServerDefinitions {
    static DEFINITIONS: OnceLock<ServerDefinitions> = OnceLock::new();
    DEFINITIONS.get_or_init(ServerDefinitions::new)
}

fn json_to_compact_string(value: &JsonValue) -> String {
    let mut out = String::new();
    append_json_value(&mut out, value);
    out
}

fn append_json_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch < '\u{20}' => {
                write!(out, "\\u{:04X}", ch as u32).expect("writing to String must not fail");
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}

fn append_json_value(out: &mut String, value: &JsonValue) {
    match value {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        JsonValue::Signed(value) => out.push_str(&value.to_string()),
        JsonValue::Unsigned(value) => out.push_str(&value.to_string()),
        JsonValue::String(value) => append_json_string(out, value),
        JsonValue::Array(values) => {
            out.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                append_json_value(out, value);
            }
            out.push(']');
        }
        JsonValue::Object(object) => {
            out.push('{');
            for (index, (key, value)) in object.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                append_json_string(out, key);
                out.push(':');
                append_json_value(out, value);
            }
            out.push('}');
        }
    }
}

fn translate_serialized_type_name(input: &str) -> String {
    let contains = |needle: &str| input.contains(needle);

    if contains("UINT") {
        if contains("512")
            || contains("384")
            || contains("256")
            || contains("192")
            || contains("160")
            || contains("128")
        {
            return input.replace("UINT", "Hash");
        }

        return input.replace("UINT", "UInt");
    }

    match input {
        "OBJECT" => "STObject".to_owned(),
        "ARRAY" => "STArray".to_owned(),
        "ACCOUNT" => "AccountID".to_owned(),
        "LEDGERENTRY" => "LedgerEntry".to_owned(),
        "NOTPRESENT" => "NotPresent".to_owned(),
        "PATHSET" => "PathSet".to_owned(),
        "VL" => "Blob".to_owned(),
        "XCHAIN_BRIDGE" => "XChainBridge".to_owned(),
        _ => input
            .split('_')
            .map(|token| {
                let mut chars = token.chars();
                match chars.next() {
                    Some(first) => {
                        let mut out = String::new();
                        out.push(first.to_ascii_uppercase());
                        for ch in chars {
                            out.push(ch.to_ascii_lowercase());
                        }
                        out
                    }
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(""),
    }
}

fn translated_type_name(ty: SerializedTypeId) -> String {
    static TYPE_NAMES: OnceLock<BTreeMap<i32, String>> = OnceLock::new();
    TYPE_NAMES
        .get_or_init(|| {
            let mut map = BTreeMap::new();
            map.insert(-1, "Done".to_owned());
            for (raw_name, type_value) in SERIALIZED_TYPE_NAME_MAP {
                let translated = translate_serialized_type_name(
                    raw_name.strip_prefix("STI_").unwrap_or(raw_name),
                );
                map.insert(*type_value, translated);
            }
            map
        })
        .get(&ty.as_i32())
        .cloned()
        .unwrap_or_else(|| "Unknown".to_owned())
}

fn json_num(value: i64) -> JsonValue {
    JsonValue::Signed(value)
}

fn json_bool(value: bool) -> JsonValue {
    JsonValue::Bool(value)
}

fn json_string(value: impl Into<String>) -> JsonValue {
    JsonValue::String(value.into())
}

fn make_field_entry(
    name: &str,
    nth: i64,
    is_vl_encoded: bool,
    is_serialized: bool,
    is_signing_field: bool,
    type_name: impl Into<String>,
) -> JsonValue {
    JsonValue::Array(vec![
        json_string(name),
        JsonValue::Object(BTreeMap::from([
            ("nth".to_owned(), json_num(nth)),
            ("isVLEncoded".to_owned(), json_bool(is_vl_encoded)),
            ("isSerialized".to_owned(), json_bool(is_serialized)),
            ("isSigningField".to_owned(), json_bool(is_signing_field)),
            ("type".to_owned(), json_string(type_name)),
        ])),
    ])
}

fn build_types() -> JsonValue {
    let mut types = BTreeMap::new();
    types.insert("Done".to_owned(), json_num(-1));

    for (raw_name, type_value) in SERIALIZED_TYPE_NAME_MAP {
        let translated =
            translate_serialized_type_name(raw_name.strip_prefix("STI_").unwrap_or(raw_name));
        types.insert(translated, json_num(i64::from(*type_value)));
    }

    JsonValue::Object(types)
}

fn build_ledger_entry_types() -> JsonValue {
    let mut ledger_entry_types = BTreeMap::new();
    ledger_entry_types.insert("Invalid".to_owned(), json_num(-1));

    for format in LedgerFormats::get_instance().iter() {
        ledger_entry_types.insert(
            format.name().to_owned(),
            json_num(i64::from(format.format_type().code())),
        );
    }

    JsonValue::Object(ledger_entry_types)
}

fn build_fields() -> JsonValue {
    let mut fields = Vec::new();

    fields.push(make_field_entry(
        "Invalid", -1, false, false, false, "Unknown",
    ));
    fields.push(make_field_entry(
        "ObjectEndMarker",
        1,
        false,
        true,
        true,
        "STObject",
    ));
    fields.push(make_field_entry(
        "ArrayEndMarker",
        1,
        false,
        true,
        true,
        "STArray",
    ));
    fields.push(make_field_entry(
        "taker_gets_funded",
        258,
        false,
        false,
        false,
        "Amount",
    ));
    fields.push(make_field_entry(
        "taker_pays_funded",
        259,
        false,
        false,
        false,
        "Amount",
    ));

    let hardcoded = [
        "Generic",
        "Invalid",
        "ObjectEndMarker",
        "ArrayEndMarker",
        "taker_gets_funded",
        "taker_pays_funded",
    ];
    let mut sorted_fields = all_sfields().iter().collect::<Vec<_>>();
    sorted_fields.sort_by_key(|field| field.code());
    for field in sorted_fields {
        if field.name().is_empty() || hardcoded.contains(&field.name()) {
            continue;
        }
        let field_type = field.field_type();
        let type_value = translated_type_name(field_type);
        fields.push(make_field_entry(
            field.name(),
            i64::from(field.field_value()),
            matches!(
                field_type,
                SerializedTypeId::VariableLength
                    | SerializedTypeId::Account
                    | SerializedTypeId::Vector256
            ),
            (field_type.as_i32() < 10000) && field.name() != "hash" && field.name() != "index",
            field.should_include(false),
            type_value,
        ));
    }

    JsonValue::Array(fields)
}

fn build_transaction_results() -> JsonValue {
    let mut results = BTreeMap::new();

    for entry in trans_results() {
        results.insert(
            entry.token.to_owned(),
            json_num(i64::from(entry.code.to_int())),
        );
    }

    JsonValue::Object(results)
}

fn build_transaction_types() -> JsonValue {
    let mut transaction_types = BTreeMap::new();
    transaction_types.insert("Invalid".to_owned(), json_num(-1));

    for format in TxFormats::get_instance().iter() {
        transaction_types.insert(
            format.name().to_owned(),
            json_num(i64::from(format.format_type().to_u16())),
        );
    }

    JsonValue::Object(transaction_types)
}

fn common_field_names(fields: &[protocol::so_template::SOElement]) -> BTreeSet<&'static str> {
    fields.iter().map(|field| field.sfield().name()).collect()
}

fn build_template_array(
    template: &[protocol::so_template::SOElement],
    common_names: &BTreeSet<&'static str>,
) -> JsonValue {
    let mut values = Vec::new();
    for element in template {
        let name = element.sfield().name();
        if common_names.contains(name) {
            continue;
        }
        values.push(JsonValue::Object(BTreeMap::from([
            ("name".to_owned(), json_string(name)),
            (
                "optionality".to_owned(),
                json_num(i64::from(element.style() as i32)),
            ),
        ])));
    }
    JsonValue::Array(values)
}

fn build_transaction_formats() -> JsonValue {
    let tx_formats = TxFormats::get_instance();
    let common_fields = tx_formats.get_common_fields();
    let common_names = common_field_names(common_fields);

    let mut formats = BTreeMap::new();
    let mut common_values = Vec::new();
    for element in common_fields {
        common_values.push(JsonValue::Object(BTreeMap::from([
            ("name".to_owned(), json_string(element.sfield().name())),
            (
                "optionality".to_owned(),
                json_num(i64::from(element.style() as i32)),
            ),
        ])));
    }
    formats.insert("common".to_owned(), JsonValue::Array(common_values));

    for format in tx_formats.iter() {
        formats.insert(
            format.name().to_owned(),
            build_template_array(format.so_template().elements(), &common_names),
        );
    }

    JsonValue::Object(formats)
}

fn build_ledger_entry_formats() -> JsonValue {
    let ledger_formats = LedgerFormats::get_instance();
    let common_fields = ledger_formats.get_common_fields();
    let common_names = common_field_names(common_fields);

    let mut formats = BTreeMap::new();
    let mut common_values = Vec::new();
    for element in common_fields {
        common_values.push(JsonValue::Object(BTreeMap::from([
            ("name".to_owned(), json_string(element.sfield().name())),
            (
                "optionality".to_owned(),
                json_num(i64::from(element.style() as i32)),
            ),
        ])));
    }
    formats.insert("common".to_owned(), JsonValue::Array(common_values));

    for format in ledger_formats.iter() {
        formats.insert(
            format.name().to_owned(),
            build_template_array(format.so_template().elements(), &common_names),
        );
    }

    JsonValue::Object(formats)
}

fn build_inner_object_formats() -> JsonValue {
    let mut formats = BTreeMap::new();

    for format in InnerObjectFormats::get_instance().iter() {
        formats.insert(
            format.name().to_owned(),
            build_template_array(format.so_template().elements(), &BTreeSet::new()),
        );
    }

    JsonValue::Object(formats)
}

fn build_transaction_flags() -> JsonValue {
    let mut flags = BTreeMap::new();
    for (name, value) in getAllTxFlags() {
        let mut inner = BTreeMap::new();
        for (flag_name, flag_value) in value {
            inner.insert(flag_name.to_owned(), json_num(i64::from(*flag_value)));
        }
        flags.insert(name.to_owned(), JsonValue::Object(inner));
    }
    JsonValue::Object(flags)
}

fn build_ledger_entry_flags() -> JsonValue {
    let mut flags = BTreeMap::new();
    for (name, value) in getAllLedgerFlags() {
        let mut inner = BTreeMap::new();
        for (flag_name, flag_value) in value {
            inner.insert(flag_name.to_owned(), json_num(i64::from(*flag_value)));
        }
        flags.insert(name.to_owned(), JsonValue::Object(inner));
    }
    JsonValue::Object(flags)
}

fn build_account_set_flags() -> JsonValue {
    let mut flags = BTreeMap::new();
    for (name, value) in getAsfFlagMap() {
        flags.insert(name.to_owned(), json_num(i64::from(*value)));
    }
    JsonValue::Object(flags)
}

pub fn do_server_definitions(params: &JsonValue) -> JsonValue {
    let requested_hash = match params {
        JsonValue::Object(object) => match object.get("hash") {
            Some(JsonValue::String(hash)) => {
                let Ok(hash) = Uint256::from_hex(hash) else {
                    return invalid_field_error("hash");
                };
                Some(hash)
            }
            Some(_) => return invalid_field_error("hash"),
            None => None,
        },
        _ => None,
    };

    let defs = server_definitions();
    if requested_hash.is_some_and(|hash| defs.hash_matches(hash)) {
        return JsonValue::Object(BTreeMap::from([(
            "hash".to_owned(),
            JsonValue::String(defs.defs_hash.to_string()),
        )]));
    }

    defs.get().clone()
}
