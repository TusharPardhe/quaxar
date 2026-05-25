//! Narrow `ledger_closed` RPC handler slice.
//!
//! This ports the the reference implementation the reference implementation shape: return the closed
//! ledger index and hash, and treat a missing closed ledger as an invariant
//! failure.

use basics::base_uint::{Uint256, to_string};
use protocol::JsonValue;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerClosed {
    pub seq: u32,
    pub hash: Uint256,
}

pub trait LedgerClosedSource {
    fn closed_ledger(&self) -> Option<LedgerClosed>;
}

pub fn do_ledger_closed<S: LedgerClosedSource>(source: &S) -> JsonValue {
    let ledger = source
        .closed_ledger()
        .expect("xrpl::doLedgerClosed : non-null closed ledger");

    let mut result = JsonValue::Object(BTreeMap::new());
    let JsonValue::Object(object) = &mut result else {
        unreachable!("result should be an object");
    };

    object.insert(
        "ledger_index".to_owned(),
        JsonValue::Unsigned(u64::from(ledger.seq)),
    );
    object.insert(
        "ledger_hash".to_owned(),
        JsonValue::String(to_string(&ledger.hash)),
    );

    result
}
