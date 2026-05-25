//! Tests for the ledger header RPC handler.

mod ledger_lookup {
    pub use rpc::ledger_lookup::*;
}

use basics::sha_map_hash::SHAMapHash;
use basics::str_hex::str_hex;
use protocol::{JsonValue, LedgerHeader};
use rpc::{LedgerHeaderResolved, LedgerHeaderSource, do_ledger_header, serialize_ledger_header};
use std::cell::Cell;
use std::collections::BTreeMap;

#[derive(Debug)]
struct FakeHeaderSource {
    resolved: Result<LedgerHeaderResolved, ledger_lookup::RpcStatus>,
    resolve_calls: Cell<u32>,
}

impl LedgerHeaderSource for FakeHeaderSource {
    fn resolve_ledger_header(&self) -> Result<LedgerHeaderResolved, ledger_lookup::RpcStatus> {
        self.resolve_calls.set(self.resolve_calls.get() + 1);
        self.resolved.clone()
    }
}

#[test]
fn ledger_header_serializes_header_and_merges_base_json() {
    let header = LedgerHeader {
        seq: 3,
        drops: 10,
        hash: SHAMapHash::new(basics::base_uint::Uint256::from_array([0x11; 32])),
        parent_hash: SHAMapHash::new(basics::base_uint::Uint256::from_array([0x22; 32])),
        tx_hash: SHAMapHash::new(basics::base_uint::Uint256::from_array([0x33; 32])),
        account_hash: SHAMapHash::new(basics::base_uint::Uint256::from_array([0x44; 32])),
        parent_close_time: 5,
        close_time: 9,
        close_time_resolution: 20,
        close_flags: 7,
        ..LedgerHeader::default()
    };

    let source = FakeHeaderSource {
        resolved: Ok(LedgerHeaderResolved {
            base_json: JsonValue::Object(BTreeMap::from([(
                "ledger_hash".to_owned(),
                JsonValue::String("base".to_owned()),
            )])),
            header,
        }),
        resolve_calls: Cell::new(0),
    };

    let result = do_ledger_header(&source);
    let JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        object.get("ledger_hash"),
        Some(&JsonValue::String("base".to_owned()))
    );
    assert_eq!(
        object.get("ledger_data"),
        Some(&JsonValue::String(str_hex(serialize_ledger_header(
            &header
        ))))
    );
    assert_eq!(serialize_ledger_header(&header).len(), 118);
    assert_eq!(source.resolve_calls.get(), 1);
}

#[test]
fn ledger_header_injects_lookup_error() {
    let source = FakeHeaderSource {
        resolved: Err(ledger_lookup::RpcStatus::new(
            ledger_lookup::RpcErrorCode::LedgerNotFound,
        )),
        resolve_calls: Cell::new(0),
    };

    let result = do_ledger_header(&source);
    let JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        object.get("error"),
        Some(&JsonValue::String("lgrNotFound".to_owned()))
    );
    assert_eq!(source.resolve_calls.get(), 1);
}
