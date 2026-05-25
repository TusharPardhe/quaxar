//! Ledger current RPC handler slice.

use protocol::JsonValue;
use std::collections::BTreeMap;

pub trait LedgerCurrentSource {
    fn current_ledger_index(&self) -> u32;
}

pub fn do_ledger_current<S: LedgerCurrentSource>(source: &S) -> JsonValue {
    let mut result = JsonValue::Object(BTreeMap::new());
    if let JsonValue::Object(object) = &mut result {
        object.insert(
            "ledger_current_index".to_owned(),
            JsonValue::Unsigned(u64::from(source.current_ledger_index())),
        );
    }
    result
}
