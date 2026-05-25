use crate::{Ledger, LedgerHeader, get_close_agree, serialize_ledger_header};
use basics::chrono::{NetClockTimePoint, to_string, to_string_iso};
use basics::str_hex::str_hex;
use protocol::{JsonOptions, JsonValue, STLedgerEntry, SerialIter, StBase};
use shamap::family::{FullBelowCache, MissingNodeReporter, SHAMapFamily, SHAMapNodeFetcher};
use shamap::item::SHAMapItem;
use shamap::traversal::TraversalError;
use std::collections::BTreeMap;
use std::hash::BuildHasher;

pub const DEFAULT_LEDGER_JSON_API_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LedgerFillOptions(u32);

impl LedgerFillOptions {
    pub const DUMP_TXRP: Self = Self(1);
    pub const DUMP_STATE: Self = Self(1 << 1);
    pub const EXPAND: Self = Self(1 << 2);
    pub const FULL: Self = Self(1 << 3);
    pub const BINARY: Self = Self(1 << 4);
    pub const OWNER_FUNDS: Self = Self(1 << 5);
    pub const DUMP_QUEUE: Self = Self(1 << 6);

    pub const fn new(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }
}

impl std::ops::BitOr for LedgerFillOptions {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for LedgerFillOptions {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LedgerFill<'a> {
    pub ledger: &'a Ledger,
    pub options: LedgerFillOptions,
    pub closed: bool,
    pub api_version: u32,
    pub close_time: Option<NetClockTimePoint>,
}

impl<'a> LedgerFill<'a> {
    pub fn new(ledger: &'a Ledger, options: LedgerFillOptions) -> Self {
        Self {
            ledger,
            options,
            closed: ledger.is_immutable(),
            api_version: DEFAULT_LEDGER_JSON_API_VERSION,
            close_time: None,
        }
    }

    pub fn with_closed(mut self, closed: bool) -> Self {
        self.closed = closed;
        self
    }

    pub fn with_api_version(mut self, api_version: u32) -> Self {
        self.api_version = api_version;
        self
    }

    pub fn with_close_time(mut self, close_time: Option<NetClockTimePoint>) -> Self {
        self.close_time = close_time;
        self
    }

    pub fn is_full(self) -> bool {
        self.options.contains(LedgerFillOptions::FULL)
    }

    pub fn is_expanded(self) -> bool {
        self.is_full() || self.options.contains(LedgerFillOptions::EXPAND)
    }

    pub fn is_binary(self) -> bool {
        self.options.contains(LedgerFillOptions::BINARY)
    }
}

pub fn fill_json_header(
    json: &mut JsonValue,
    closed: bool,
    info: &LedgerHeader,
    full: bool,
    api_version: u32,
) {
    let object = ensure_object(json);
    object.insert(
        "parent_hash".to_owned(),
        JsonValue::String(info.parent_hash.to_string()),
    );
    object.insert(
        "ledger_index".to_owned(),
        if api_version > 1 {
            JsonValue::Unsigned(u64::from(info.seq))
        } else {
            JsonValue::String(info.seq.to_string())
        },
    );

    if closed {
        object.insert("closed".to_owned(), JsonValue::Bool(true));
    } else if !full {
        object.insert("closed".to_owned(), JsonValue::Bool(false));
        return;
    }

    object.insert(
        "ledger_hash".to_owned(),
        JsonValue::String(info.hash.to_string()),
    );
    object.insert(
        "transaction_hash".to_owned(),
        JsonValue::String(info.tx_hash.to_string()),
    );
    object.insert(
        "account_hash".to_owned(),
        JsonValue::String(info.account_hash.to_string()),
    );
    object.insert(
        "total_coins".to_owned(),
        JsonValue::String(info.drops.to_string()),
    );
    object.insert(
        "close_flags".to_owned(),
        JsonValue::Unsigned(u64::from(info.close_flags)),
    );
    object.insert(
        "parent_close_time".to_owned(),
        JsonValue::Unsigned(u64::from(info.parent_close_time)),
    );
    object.insert(
        "close_time".to_owned(),
        JsonValue::Unsigned(u64::from(info.close_time)),
    );
    object.insert(
        "close_time_resolution".to_owned(),
        JsonValue::Unsigned(u64::from(info.close_time_resolution)),
    );

    if info.close_time != 0 {
        let close_time = NetClockTimePoint::from(info.close_time);
        object.insert(
            "close_time_human".to_owned(),
            JsonValue::String(to_string(close_time)),
        );
        if !get_close_agree(info) {
            object.insert("close_time_estimated".to_owned(), JsonValue::Bool(true));
        }
        object.insert(
            "close_time_iso".to_owned(),
            JsonValue::String(to_string_iso(close_time)),
        );
    }
}

pub fn fill_json_binary(json: &mut JsonValue, closed: bool, info: &LedgerHeader) {
    let object = ensure_object(json);
    if !closed {
        object.insert("closed".to_owned(), JsonValue::Bool(false));
        return;
    }

    object.insert("closed".to_owned(), JsonValue::Bool(true));
    object.insert(
        "ledger_data".to_owned(),
        JsonValue::String(str_hex(serialize_ledger_header(info, false))),
    );
}

pub fn fill_json_state(json: &mut JsonValue, fill: &LedgerFill<'_>) -> Result<(), TraversalError> {
    let object = ensure_object(json);
    let mut state = Vec::new();
    let expanded = fill.is_expanded();
    let binary = fill.is_binary();

    fill.ledger
        .state_map()
        .visit_leaves(&mut |_| None, &mut |item| {
            state.push(state_leaf_json(item, binary, expanded));
        })?;

    object.insert("accountState".to_owned(), JsonValue::Array(state));
    Ok(())
}

pub fn fill_json_state_with_family<CLOCK, S, C, F, MR, NS>(
    json: &mut JsonValue,
    fill: &LedgerFill<'_>,
    family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
) -> Result<(), TraversalError>
where
    CLOCK: basics::tagged_cache::CacheClock,
    S: BuildHasher + Clone,
    C: FullBelowCache,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    let object = ensure_object(json);
    let mut state = Vec::new();
    let expanded = fill.is_expanded();
    let binary = fill.is_binary();

    fill.ledger
        .state_map()
        .visit_leaves_with_family(family, &mut |item| {
            state.push(state_leaf_json(item, binary, expanded));
        })?;

    object.insert("accountState".to_owned(), JsonValue::Array(state));
    Ok(())
}

pub fn fill_json(json: &mut JsonValue, fill: &LedgerFill<'_>) -> Result<(), TraversalError> {
    if fill.is_binary() {
        fill_json_binary(json, fill.closed, &fill.ledger.header());
    } else {
        fill_json_header(
            json,
            fill.closed,
            &fill.ledger.header(),
            fill.is_full(),
            fill.api_version,
        );
    }

    if fill.is_full() || fill.options.contains(LedgerFillOptions::DUMP_STATE) {
        fill_json_state(json, fill)?;
    }

    Ok(())
}

pub fn fill_json_with_family<CLOCK, S, C, F, MR, NS>(
    json: &mut JsonValue,
    fill: &LedgerFill<'_>,
    family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
) -> Result<(), TraversalError>
where
    CLOCK: basics::tagged_cache::CacheClock,
    S: BuildHasher + Clone,
    C: FullBelowCache,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    if fill.is_binary() {
        fill_json_binary(json, fill.closed, &fill.ledger.header());
    } else {
        fill_json_header(
            json,
            fill.closed,
            &fill.ledger.header(),
            fill.is_full(),
            fill.api_version,
        );
    }

    if fill.is_full() || fill.options.contains(LedgerFillOptions::DUMP_STATE) {
        fill_json_state_with_family(json, fill, family)?;
    }

    Ok(())
}

pub fn add_json(json: &mut JsonValue, fill: &LedgerFill<'_>) -> Result<(), TraversalError> {
    let root = ensure_object(json);
    let mut ledger = JsonValue::Object(BTreeMap::new());
    fill_json(&mut ledger, fill)?;
    root.insert("ledger".to_owned(), ledger);
    Ok(())
}

pub fn add_json_with_family<CLOCK, S, C, F, MR, NS>(
    json: &mut JsonValue,
    fill: &LedgerFill<'_>,
    family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
) -> Result<(), TraversalError>
where
    CLOCK: basics::tagged_cache::CacheClock,
    S: BuildHasher + Clone,
    C: FullBelowCache,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    let root = ensure_object(json);
    let mut ledger = JsonValue::Object(BTreeMap::new());
    fill_json_with_family(&mut ledger, fill, family)?;
    root.insert("ledger".to_owned(), ledger);
    Ok(())
}

pub fn get_json(fill: &LedgerFill<'_>) -> Result<JsonValue, TraversalError> {
    let mut json = JsonValue::Null;
    fill_json(&mut json, fill)?;
    Ok(json)
}

pub fn get_json_with_family<CLOCK, S, C, F, MR, NS>(
    fill: &LedgerFill<'_>,
    family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
) -> Result<JsonValue, TraversalError>
where
    CLOCK: basics::tagged_cache::CacheClock,
    S: BuildHasher + Clone,
    C: FullBelowCache,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    let mut json = JsonValue::Null;
    fill_json_with_family(&mut json, fill, family)?;
    Ok(json)
}

pub fn copy_from(to: &mut JsonValue, from: &JsonValue) {
    if matches!(to, JsonValue::Null) {
        *to = from.clone();
        return;
    }

    let JsonValue::Object(target) = to else {
        panic!("copy_from target must be an object or null");
    };

    let JsonValue::Object(source) = from else {
        assert!(
            matches!(from, JsonValue::Null),
            "copy_from source must be an object or null"
        );
        return;
    };

    for (key, value) in source {
        target.insert(key.clone(), value.clone());
    }
}

fn ensure_object(json: &mut JsonValue) -> &mut BTreeMap<String, JsonValue> {
    if matches!(json, JsonValue::Null) {
        *json = JsonValue::Object(BTreeMap::new());
    }

    let JsonValue::Object(object) = json else {
        panic!("ledger json root must be an object or null");
    };
    object
}

fn state_leaf_json(item: &SHAMapItem, binary: bool, expanded: bool) -> JsonValue {
    if binary {
        return JsonValue::Object(BTreeMap::from([
            ("hash".to_owned(), JsonValue::String(item.key().to_string())),
            (
                "tx_blob".to_owned(),
                JsonValue::String(str_hex(item.data())),
            ),
        ]));
    }

    if expanded {
        let mut serial = SerialIter::new(item.data());
        let entry = STLedgerEntry::from_serial_iter(&mut serial, item.key());
        return entry.json(JsonOptions::NONE);
    }

    JsonValue::String(item.key().to_string())
}
