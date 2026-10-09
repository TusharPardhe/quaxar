//! Direct JSON writer for serialized types.
//!
//! `StBase::json()` builds a `JsonValue` tree (a `BTreeMap` node and owned
//! `String` key per field) that the RPC layer then serializes and drops.
//! Profiling showed building that tree was most of the cost of rendering a
//! ledger object. This module writes the same JSON text straight into a
//! byte buffer instead.
//!
//! Output is byte-identical to `sonic_rs::to_vec(&value.json(options))`:
//! keys in lexicographic order, the same per-field value rules as each
//! type's `json()`, and the same string escaping (any string needing
//! escapes is delegated to sonic-rs). That equivalence is enforced by the
//! unit tests below and by the ledger corpus test in
//! `tests/serialization/json_writer_corpus.rs`.
//!
//! Key order uses the template slot permutation precomputed in
//! [`SOTemplate::json_order`]: a templated object's fields are stored in
//! template order, so emitting them in sorted-key order is a table walk with
//! no comparisons. Free (untemplated) objects sort their few fields.
//!
//! Types without a dedicated writer (e.g. `STNumber`, `STPathSet`,
//! `STXChainBridge`) fall back to `json()` for that field only.

use crate::keylet::ledger_entry_type_from_code;
use crate::ter::trans_token;
use crate::{
    Asset, JsonOptions, JsonValue, LedgerEntryType, LedgerFormats, RawJson, SField, STAccount,
    STAmount, STArray, STBlob, STInt32, STLedgerEntry, STObject, STTx, STUInt8, STUInt16, STUInt32,
    STUInt64, STUInt128, STUInt160, STUInt192, STUInt256, STVector256, SerializedTypeId, StBase,
    Ter, TxFormats, TxType, currency_to_string, get_field_by_symbol, is_xrp_currency,
};
use basics::str_hex::encode_upper_to_slice;

/// A key/value appended to an object by a caller (for example `index` on a
/// ledger entry, or computed fields an RPC handler adds). `value` must be a
/// complete JSON value.
#[derive(Debug, Clone, Copy)]
pub struct ExtraField<'a> {
    pub key: &'a str,
    pub value: &'a [u8],
}

thread_local! {
    static RAW_RENDERING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` with raw rendering enabled on this thread: helpers such as
/// [`ledger_entry_json`] then return [`JsonValue::Raw`] (pre-serialized,
/// no tree) instead of a `JsonValue` tree. The RPC server enables this
/// around handler dispatch, where results are only serialized; library
/// callers and tests that inspect results get trees by default.
pub fn with_raw_rendering<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            RAW_RENDERING.with(|flag| flag.set(self.0));
        }
    }
    let _restore = Restore(RAW_RENDERING.with(|flag| flag.replace(true)));
    f()
}

/// Whether [`with_raw_rendering`] is active on this thread.
pub fn raw_rendering_enabled() -> bool {
    RAW_RENDERING.with(std::cell::Cell::get)
}

/// `sle.json(JsonOptions::NONE)`, as a raw value when raw rendering is on.
pub fn ledger_entry_json(sle: &STLedgerEntry) -> JsonValue {
    if raw_rendering_enabled() {
        to_raw_json(sle, JsonOptions::NONE)
    } else {
        sle.json(JsonOptions::NONE)
    }
}

/// `sle.json(JsonOptions::NONE)` with `extras` inserted (later extras win),
/// as a raw value when raw rendering is on.
pub fn ledger_entry_json_extended(
    sle: &STLedgerEntry,
    extras: Vec<(&str, JsonValue)>,
) -> JsonValue {
    if raw_rendering_enabled() {
        let rendered: Vec<(&str, Vec<u8>)> = extras
            .iter()
            .map(|(key, value)| {
                (
                    *key,
                    sonic_rs::to_vec(value).expect("serializing a JsonValue cannot fail"),
                )
            })
            .collect();
        let fields: Vec<ExtraField<'_>> = rendered
            .iter()
            .map(|(key, value)| ExtraField { key, value })
            .collect();
        let mut out = Vec::with_capacity(768);
        ledger_entry_json_with(sle, JsonOptions::NONE, &fields, &mut out);
        JsonValue::Raw(RawJson::from_trusted_bytes(out))
    } else {
        let mut json = sle.json(JsonOptions::NONE);
        if let JsonValue::Object(object) = &mut json {
            for (key, value) in extras {
                object.insert(key.to_owned(), value);
            }
        }
        json
    }
}

/// `tx.json(options)` with `extras` inserted (later extras win), as a raw
/// value when raw rendering is on. For callers that only add keys.
pub fn transaction_json_extended(
    tx: &STTx,
    options: JsonOptions,
    extras: Vec<(&str, JsonValue)>,
) -> JsonValue {
    if raw_rendering_enabled() {
        let rendered: Vec<(&str, Vec<u8>)> = extras
            .iter()
            .map(|(key, value)| {
                (
                    *key,
                    sonic_rs::to_vec(value).expect("serializing a JsonValue cannot fail"),
                )
            })
            .collect();
        let fields: Vec<ExtraField<'_>> = rendered
            .iter()
            .map(|(key, value)| ExtraField { key, value })
            .collect();
        let mut out = Vec::with_capacity(1024);
        transaction_json_with(tx, options, &fields, &mut out);
        JsonValue::Raw(RawJson::from_trusted_bytes(out))
    } else {
        let mut json = tx.json(options);
        if let JsonValue::Object(object) = &mut json {
            for (key, value) in extras {
                object.insert(key.to_owned(), value);
            }
        }
        json
    }
}

/// `value.json(options)` for an object, transaction, or ledger entry. With
/// raw rendering on, the top level stays a `JsonValue::Object` (so callers
/// can still add, remove, or read top-level keys and scalar values) while
/// nested objects and arrays (metadata `AffectedNodes`, `Memos`, `Signers`,
/// ...) are pre-rendered `Raw` values. Ledger entries render fully raw.
pub fn shallow_json(value: &dyn StBase, options: JsonOptions) -> JsonValue {
    if !raw_rendering_enabled() {
        return value.json(options);
    }
    let any = value.as_any();
    if let Some(tx) = any.downcast_ref::<STTx>() {
        // STTx::json renders its object with JsonOptions::NONE.
        let mut object = shallow_fields(tx, JsonOptions::NONE);
        if (options & JsonOptions::DISABLE_API_PRIOR_V2) == JsonOptions::NONE {
            object.insert(
                "hash".to_string(),
                JsonValue::String(tx.get_transaction_id().to_string()),
            );
        }
        return JsonValue::Object(object);
    }
    if any.is::<STLedgerEntry>() {
        return to_raw_json(value, options);
    }
    if let Some(object) = any.downcast_ref::<STObject>() {
        return JsonValue::Object(shallow_fields(object, options));
    }
    value.json(options)
}

fn shallow_fields(
    object: &STObject,
    options: JsonOptions,
) -> std::collections::BTreeMap<String, JsonValue> {
    let (fields, _) = object.json_parts();
    let mut map = std::collections::BTreeMap::new();
    for field in fields {
        let value = field.get();
        let json = match value.stype() {
            SerializedTypeId::NotPresent => continue,
            SerializedTypeId::Object | SerializedTypeId::Array => to_raw_json(value, options),
            _ => value.json(options),
        };
        map.insert(value.fname().name().to_string(), json);
    }
    map
}

/// Render `value` exactly as `sonic_rs::to_vec(&value.json(options))`.
pub fn to_json_bytes(value: &dyn StBase, options: JsonOptions) -> Vec<u8> {
    let mut out = Vec::with_capacity(512);
    write_value(value, options, &mut out);
    out
}

/// Render `value` as a [`JsonValue::Raw`] that serializes to the same bytes as
/// `value.json(options)`.
pub fn to_raw_json(value: &dyn StBase, options: JsonOptions) -> JsonValue {
    JsonValue::Raw(RawJson::from_trusted_bytes(to_json_bytes(value, options)))
}

/// Render a ledger entry with additional top-level keys merged in sorted
/// position, as if they had been inserted into `sle.json(options)`.
/// `extras` keys must not collide with the entry's own keys; on collision the
/// extra wins, matching `BTreeMap::insert`.
pub fn ledger_entry_json_with(
    sle: &STLedgerEntry,
    options: JsonOptions,
    extras: &[ExtraField<'_>],
    out: &mut Vec<u8>,
) {
    let mut key_buf = [0_u8; 64];
    let index = sle.key();
    encode_upper_to_slice(index.data(), &mut key_buf);
    let mut index_json = [0_u8; 66];
    index_json[0] = b'"';
    index_json[1..65].copy_from_slice(&key_buf);
    index_json[65] = b'"';

    let mut mpt_json = Vec::new();
    let mut builtin = [
        ExtraField {
            key: "index",
            value: &index_json,
        },
        ExtraField {
            key: "",
            value: &[],
        },
    ];
    let mut builtin_len = 1;
    if sle.get_type() == LedgerEntryType::MPTokenIssuance {
        let id = crate::make_mpt_id(
            sle.get_field_u32(get_field_by_symbol("sfSequence")),
            sle.get_account_id(get_field_by_symbol("sfIssuer")),
        );
        write_str(&mut mpt_json, &id.to_string());
        builtin[1] = ExtraField {
            key: "mpt_issuance_id",
            value: &mpt_json,
        };
        builtin_len = 2;
    }

    // Merge builtin and caller extras; caller extras win on equal keys.
    let mut merged: Vec<ExtraField<'_>> = Vec::with_capacity(builtin_len + extras.len());
    merged.extend_from_slice(&builtin[..builtin_len]);
    for extra in extras {
        if let Some(slot) = merged.iter_mut().find(|existing| existing.key == extra.key) {
            *slot = *extra;
        } else {
            merged.push(*extra);
        }
    }
    merged.sort_unstable_by(|a, b| a.key.cmp(b.key));
    write_object(sle, options, &merged, out);
}

/// Render a transaction (`STTx::json`: fields plus `hash` unless
/// `DISABLE_API_PRIOR_V2`) with additional keys merged in sorted position.
/// Caller extras win on key collisions.
pub fn transaction_json_with(
    tx: &STTx,
    options: JsonOptions,
    extras: &[ExtraField<'_>],
    out: &mut Vec<u8>,
) {
    let mut hash_json = [b'"'; 66];
    encode_upper_to_slice(tx.get_transaction_id().data(), &mut hash_json[1..65]);
    let mut merged: Vec<ExtraField<'_>> = Vec::with_capacity(1 + extras.len());
    if (options & JsonOptions::DISABLE_API_PRIOR_V2) == JsonOptions::NONE {
        merged.push(ExtraField {
            key: "hash",
            value: &hash_json,
        });
    }
    for extra in extras {
        if let Some(slot) = merged.iter_mut().find(|existing| existing.key == extra.key) {
            *slot = *extra;
        } else {
            merged.push(*extra);
        }
    }
    merged.sort_unstable_by(|a, b| a.key.cmp(b.key));
    // STTx::json renders its object with JsonOptions::NONE.
    write_object(tx, JsonOptions::NONE, &merged, out);
}

/// Render an object (`STObject::json`) with additional keys merged in sorted
/// position. `extras` must be sorted by key and unique; an extra replaces a
/// field with the same name.
pub fn object_json_with(
    object: &STObject,
    options: JsonOptions,
    extras: &[ExtraField<'_>],
    out: &mut Vec<u8>,
) {
    debug_assert!(extras.windows(2).all(|w| w[0].key < w[1].key));
    write_object(object, options, extras, out);
}

/// Append the JSON for `value`.
pub fn write_value(value: &dyn StBase, options: JsonOptions, out: &mut Vec<u8>) {
    let any = value.as_any();
    match value.stype() {
        SerializedTypeId::Object => {
            if let Some(object) = any.downcast_ref::<STObject>() {
                return write_object(object, options, &[], out);
            }
        }
        SerializedTypeId::LedgerEntry => {
            if let Some(sle) = any.downcast_ref::<STLedgerEntry>() {
                return ledger_entry_json_with(sle, options, &[], out);
            }
        }
        SerializedTypeId::Transaction => {
            if let Some(tx) = any.downcast_ref::<STTx>() {
                return transaction_json_with(tx, options, &[], out);
            }
        }
        SerializedTypeId::Array => {
            if let Some(array) = any.downcast_ref::<STArray>() {
                return write_array(array, options, out);
            }
        }
        SerializedTypeId::UInt8 => {
            if let Some(int) = any.downcast_ref::<STUInt8>() {
                if int.fname() == sf_transaction_result() {
                    return write_str(out, trans_token(Ter::from_int(i32::from(int.value()))));
                }
                return write_u64(out, u64::from(int.value()));
            }
        }
        SerializedTypeId::UInt16 => {
            if let Some(int) = any.downcast_ref::<STUInt16>() {
                let field = int.fname();
                let code = int.value();
                if field == sf_ledger_entry_type()
                    && let Some(entry_type) = ledger_entry_type_from_code(code)
                    && let Some(item) = LedgerFormats::get_instance().find_by_type(entry_type)
                {
                    return write_str(out, item.name());
                }
                if field == sf_transaction_type()
                    && let Some(item) =
                        TxFormats::get_instance().find_by_type(TxType::from_u16(code))
                {
                    return write_str(out, item.name());
                }
                return write_u64(out, u64::from(code));
            }
        }
        SerializedTypeId::UInt32 => {
            if let Some(int) = any.downcast_ref::<STUInt32>()
                && int.fname() != sf_permission_value()
            {
                return write_u64(out, u64::from(int.value()));
            }
        }
        SerializedTypeId::UInt64 => {
            if let Some(int) = any.downcast_ref::<STUInt64>() {
                let value = int.value();
                out.push(b'"');
                if int.fname().should_meta(SField::S_MD_BASE_TEN) {
                    write_u64(out, value);
                } else {
                    write_lower_hex(out, value);
                }
                out.push(b'"');
                return;
            }
        }
        SerializedTypeId::Int32 => {
            if let Some(int) = any.downcast_ref::<STInt32>() {
                return write_i64(out, i64::from(int.value()));
            }
        }
        SerializedTypeId::UInt128 => {
            if let Some(bits) = any.downcast_ref::<STUInt128>() {
                return write_hex_str(out, bits.value().data());
            }
        }
        SerializedTypeId::UInt160 => {
            if let Some(bits) = any.downcast_ref::<STUInt160>() {
                return write_hex_str(out, bits.value().data());
            }
        }
        SerializedTypeId::UInt192 => {
            if let Some(bits) = any.downcast_ref::<STUInt192>() {
                return write_hex_str(out, bits.value().data());
            }
        }
        SerializedTypeId::UInt256 => {
            if let Some(bits) = any.downcast_ref::<STUInt256>() {
                return write_hex_str(out, bits.value().data());
            }
        }
        SerializedTypeId::VariableLength => {
            if let Some(blob) = any.downcast_ref::<STBlob>() {
                return write_hex_str(out, blob.data());
            }
        }
        SerializedTypeId::Account => {
            if let Some(account) = any.downcast_ref::<STAccount>() {
                if account.is_default() {
                    out.extend_from_slice(b"\"\"");
                } else {
                    write_account(out, *account.value());
                }
                return;
            }
        }
        SerializedTypeId::Amount => {
            if let Some(amount) = any.downcast_ref::<STAmount>() {
                return write_amount(amount, out);
            }
        }
        SerializedTypeId::Vector256 => {
            if let Some(vector) = any.downcast_ref::<STVector256>() {
                out.push(b'[');
                for (i, hash) in vector.value().iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    write_hex_str(out, hash.data());
                }
                out.push(b']');
                return;
            }
        }
        _ => {}
    }
    write_fallback(value, options, out);
}

/// Types (and special-cased fields) without a dedicated writer: render the
/// existing tree for this value only.
fn write_fallback(value: &dyn StBase, options: JsonOptions, out: &mut Vec<u8>) {
    sonic_rs::to_writer(&mut *out, &value.json(options))
        .expect("serializing a JsonValue into a Vec cannot fail");
}

fn write_amount(amount: &STAmount, out: &mut Vec<u8>) {
    // STAmount::set_json: native -> string; otherwise an object with
    // `value` plus the asset keys. Keys below are already in sorted order.
    if amount.native() {
        return write_str(out, &amount.text());
    }
    match amount.asset() {
        Asset::Issue(issue) => {
            out.extend_from_slice(b"{\"currency\":");
            write_str(out, &currency_to_string(issue.currency));
            if !is_xrp_currency(issue.currency) {
                out.extend_from_slice(b",\"issuer\":");
                write_account(out, issue.account);
            }
        }
        Asset::MPTIssue(issue) => {
            out.extend_from_slice(b"{\"mpt_issuance_id\":");
            write_str(out, &issue.text());
        }
    }
    out.extend_from_slice(b",\"value\":");
    write_str(out, &amount.text());
    out.push(b'}');
}

fn write_array(array: &STArray, options: JsonOptions, out: &mut Vec<u8>) {
    out.push(b'[');
    let mut first = true;
    for object in array.iter() {
        if object.stype() == SerializedTypeId::NotPresent {
            continue;
        }
        if !first {
            out.push(b',');
        }
        first = false;
        out.extend_from_slice(b"{\"");
        out.extend_from_slice(object.fname().name().as_bytes());
        out.extend_from_slice(b"\":");
        write_object(object, options, &[], out);
        out.push(b'}');
    }
    out.push(b']');
}

/// Maximum fields rendered through the stack order buffer; larger objects
/// (none exist in current formats) use a heap buffer.
const STACK_FIELDS: usize = 64;

/// Write `object`'s present fields plus `extras` (sorted by key) as a JSON
/// object with sorted keys.
fn write_object(
    object: &STObject,
    options: JsonOptions,
    extras: &[ExtraField<'_>],
    out: &mut Vec<u8>,
) {
    let (fields, template) = object.json_parts();

    // Template fast path: fields are stored in template order, so the
    // precomputed slot permutation is the sorted-key order.
    let template_order = template.filter(|template| {
        template.size() == fields.len()
            && template
                .elements()
                .iter()
                .zip(fields)
                .all(|(element, field)| std::ptr::eq(element.sfield(), field.get().fname()))
    });

    let mut stack = [0_u16; STACK_FIELDS];
    let mut heap = Vec::new();
    let order: &[u16] = if let Some(template) = template_order {
        template.json_order()
    } else {
        let indices: &mut [u16] = if fields.len() <= STACK_FIELDS {
            for (i, slot) in stack[..fields.len()].iter_mut().enumerate() {
                *slot = i as u16;
            }
            &mut stack[..fields.len()]
        } else {
            heap.extend(0..fields.len() as u16);
            &mut heap
        };
        indices.sort_unstable_by_key(|i| fields[usize::from(*i)].get().fname().name());
        // A free object with two fields of the same name collapses to one
        // key in the tree (last inserted wins); mirror that rarely-hit case
        // through the tree path.
        if indices.windows(2).any(|w| {
            fields[usize::from(w[0])].get().fname() == fields[usize::from(w[1])].get().fname()
        }) {
            return write_fallback(object, options, out);
        }
        indices
    };

    out.push(b'{');
    let mut first = true;
    let mut extras = extras.iter().peekable();
    for index in order {
        let value = fields[usize::from(*index)].get();
        if value.stype() == SerializedTypeId::NotPresent {
            continue;
        }
        let name = value.fname().name();
        while let Some(extra) = extras.next_if(|extra| extra.key < name) {
            write_entry(out, &mut first, extra.key, |out| {
                out.extend_from_slice(extra.value)
            });
        }
        if extras.peek().is_some_and(|extra| extra.key == name) {
            // BTreeMap::insert after json(): the extra replaces the field.
            continue;
        }
        // SField names are plain identifiers: no escaping check needed.
        debug_assert!(name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
        if !first {
            out.push(b',');
        }
        first = false;
        out.reserve(name.len() + 3);
        out.push(b'"');
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b"\":");
        write_value(value, options, out);
    }
    for extra in extras {
        write_entry(out, &mut first, extra.key, |out| {
            out.extend_from_slice(extra.value)
        });
    }
    out.push(b'}');
}

#[inline]
fn write_entry(out: &mut Vec<u8>, first: &mut bool, key: &str, value: impl FnOnce(&mut Vec<u8>)) {
    if !*first {
        out.push(b',');
    }
    *first = false;
    write_str(out, key);
    out.push(b':');
    value(out);
}

fn sf_transaction_result() -> &'static SField {
    static FIELD: std::sync::OnceLock<&'static SField> = std::sync::OnceLock::new();
    FIELD.get_or_init(|| get_field_by_symbol("sfTransactionResult"))
}

fn sf_ledger_entry_type() -> &'static SField {
    static FIELD: std::sync::OnceLock<&'static SField> = std::sync::OnceLock::new();
    FIELD.get_or_init(|| get_field_by_symbol("sfLedgerEntryType"))
}

fn sf_transaction_type() -> &'static SField {
    static FIELD: std::sync::OnceLock<&'static SField> = std::sync::OnceLock::new();
    FIELD.get_or_init(|| get_field_by_symbol("sfTransactionType"))
}

fn sf_permission_value() -> &'static SField {
    static FIELD: std::sync::OnceLock<&'static SField> = std::sync::OnceLock::new();
    FIELD.get_or_init(|| get_field_by_symbol("sfPermissionValue"))
}

// ---------------------------------------------------------------------------
// Scalar writers
// ---------------------------------------------------------------------------

/// JSON string. Strings that need escaping are delegated to sonic-rs so the
/// escape sequences match the tree serializer exactly.
#[inline]
pub fn write_str(out: &mut Vec<u8>, text: &str) {
    if text
        .bytes()
        .all(|byte| byte >= 0x20 && byte != b'"' && byte != b'\\')
    {
        out.reserve(text.len() + 2);
        out.push(b'"');
        out.extend_from_slice(text.as_bytes());
        out.push(b'"');
    } else {
        sonic_rs::to_writer(&mut *out, text).expect("serializing a str into a Vec cannot fail");
    }
}

#[inline]
fn write_account(out: &mut Vec<u8>, account: crate::AccountID) {
    crate::account_id::with_base58(account, |text| write_ascii_str(out, text));
}

/// String known to contain only escape-free ASCII (base58, hex, digits).
#[inline]
fn write_ascii_str(out: &mut Vec<u8>, text: &str) {
    debug_assert!(text.bytes().all(|b| b.is_ascii_alphanumeric()));
    out.reserve(text.len() + 2);
    out.push(b'"');
    out.extend_from_slice(text.as_bytes());
    out.push(b'"');
}

#[inline]
fn write_hex_str(out: &mut Vec<u8>, bytes: &[u8]) {
    let start = out.len();
    out.resize(start + bytes.len() * 2 + 2, b'"');
    encode_upper_to_slice(bytes, &mut out[start + 1..start + 1 + bytes.len() * 2]);
}

#[inline]
fn write_u64(out: &mut Vec<u8>, value: u64) {
    let mut buffer = itoa::Buffer::new();
    out.extend_from_slice(buffer.format(value).as_bytes());
}

#[inline]
fn write_i64(out: &mut Vec<u8>, value: i64) {
    let mut buffer = itoa::Buffer::new();
    out.extend_from_slice(buffer.format(value).as_bytes());
}

/// Lowercase hex without leading zeros (`format!("{value:x}")`).
#[inline]
fn write_lower_hex(out: &mut Vec<u8>, value: u64) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    if value == 0 {
        out.push(b'0');
        return;
    }
    let digits = (64 - value.leading_zeros()).div_ceil(4) as usize;
    for i in (0..digits).rev() {
        out.push(DIGITS[((value >> (i * 4)) & 0xF) as usize]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AccountID, Issue, STLedgerEntry, SerialIter, currency_from_string,
        get_field_by_symbol as f, offer_keylet,
    };

    fn tree_bytes(value: &dyn StBase) -> Vec<u8> {
        sonic_rs::to_vec(&value.json(JsonOptions::NONE)).unwrap()
    }

    fn assert_same(value: &dyn StBase) {
        let direct = to_json_bytes(value, JsonOptions::NONE);
        assert_eq!(
            String::from_utf8(direct).unwrap(),
            String::from_utf8(tree_bytes(value)).unwrap()
        );
    }

    fn sample_offer() -> STLedgerEntry {
        let owner = AccountID::from_array([7; 20]);
        let issuer = AccountID::from_array([9; 20]);
        let key = offer_keylet(
            basics::base_uint::Uint160::from_slice(owner.data()).unwrap(),
            42,
        )
        .key;
        let mut offer = STLedgerEntry::from_type_and_key(LedgerEntryType::Offer, key);
        offer.set_account_id(f("sfAccount"), owner);
        offer.set_field_u32(f("sfSequence"), 42);
        offer.set_field_u32(f("sfFlags"), 0x0002_0000);
        offer.set_field_amount(
            f("sfTakerPays"),
            STAmount::new_with_asset(
                f("sfTakerPays"),
                Issue::new(currency_from_string("USD"), issuer),
                3_076_560_200_591_725,
                -15,
                false,
            ),
        );
        offer.set_field_amount(f("sfTakerGets"), STAmount::new_native(6_000_000, false));
        offer.set_field_h256(f("sfBookDirectory"), key);
        offer.set_field_u64(f("sfBookNode"), 0x1234_ABCD);
        offer.set_field_u64(f("sfOwnerNode"), 0);
        offer.set_field_h256(f("sfPreviousTxnID"), key);
        offer.set_field_u32(f("sfPreviousTxnLgrSeq"), 21_305_409);
        offer
    }

    #[test]
    fn offer_matches_tree_serialization() {
        let offer = sample_offer();
        assert_same(&offer);
        // Round trip through the wire format (decoded objects carry the
        // template, exercising the slot-order fast path).
        let bytes = offer.get_serializer().data().to_vec();
        let decoded = STLedgerEntry::from_serial_iter(&mut SerialIter::new(&bytes), *offer.key());
        assert_same(&decoded);
    }

    #[test]
    fn extras_merge_in_sorted_position_and_override() {
        let offer = sample_offer();
        let mut tree = match offer.json(JsonOptions::NONE) {
            JsonValue::Object(object) => object,
            _ => unreachable!(),
        };
        tree.insert("owner_funds".into(), JsonValue::String("100".into()));
        tree.insert("Flags".into(), JsonValue::Unsigned(7));
        tree.insert("aaa".into(), JsonValue::Null);
        let expected = sonic_rs::to_vec(&JsonValue::Object(tree)).unwrap();
        let mut out = Vec::new();
        ledger_entry_json_with(
            &offer,
            JsonOptions::NONE,
            &[
                ExtraField {
                    key: "owner_funds",
                    value: b"\"100\"",
                },
                ExtraField {
                    key: "Flags",
                    value: b"7",
                },
                ExtraField {
                    key: "aaa",
                    value: b"null",
                },
            ],
            &mut out,
        );
        assert_eq!(
            String::from_utf8(out).unwrap(),
            String::from_utf8(expected).unwrap()
        );
    }

    #[test]
    fn strings_needing_escapes_match_sonic() {
        for text in [
            "plain",
            "q\"uote",
            "back\\slash",
            "tab\tnl\n",
            "\u{1}\u{1f}",
            "é ü 漢",
        ] {
            let mut out = Vec::new();
            write_str(&mut out, text);
            assert_eq!(out, sonic_rs::to_vec(text).unwrap(), "{text:?}");
        }
    }

    #[test]
    fn lower_hex_matches_format() {
        for value in [0, 1, 0xF, 0x10, 0xABC, u64::MAX, 1 << 63, 0x1234_5678_9ABC] {
            let mut out = Vec::new();
            write_lower_hex(&mut out, value);
            assert_eq!(String::from_utf8(out).unwrap(), format!("{value:x}"));
        }
    }

    #[test]
    fn transaction_matches_tree_for_both_hash_modes() {
        let account = AccountID::from_array([3; 20]);
        let tx = crate::STTx::new(TxType::PAYMENT, |object| {
            object.set_account_id(f("sfAccount"), account);
            object.set_account_id(f("sfDestination"), AccountID::from_array([4; 20]));
            object.set_field_amount(f("sfAmount"), STAmount::new_native(1_000_000, false));
            object.set_field_amount(f("sfFee"), STAmount::new_native(12, false));
            object.set_field_u32(f("sfSequence"), 9);
            object.set_field_vl(f("sfSigningPubKey"), &[0x02; 33]);
        });
        for options in [JsonOptions::NONE, JsonOptions::DISABLE_API_PRIOR_V2] {
            let direct = to_json_bytes(&tx, options);
            let tree = sonic_rs::to_vec(&tx.json(options)).unwrap();
            assert_eq!(
                String::from_utf8(direct).unwrap(),
                String::from_utf8(tree).unwrap()
            );
        }
    }

    #[test]
    fn raw_json_serializes_verbatim_with_sonic_and_correctly_elsewhere() {
        let offer = sample_offer();
        let raw = to_raw_json(&offer, JsonOptions::NONE);
        let wrapped = JsonValue::Array(vec![raw.clone(), JsonValue::Unsigned(1)]);
        let expected =
            JsonValue::Array(vec![offer.json(JsonOptions::NONE), JsonValue::Unsigned(1)]);
        assert_eq!(
            sonic_rs::to_vec(&wrapped).unwrap(),
            sonic_rs::to_vec(&expected).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&wrapped).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        let JsonValue::Raw(raw) = raw else {
            unreachable!()
        };
        assert_eq!(raw.to_tree(), offer.json(JsonOptions::NONE));
    }
}
