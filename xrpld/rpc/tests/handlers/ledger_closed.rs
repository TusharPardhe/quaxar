//! Tests for the ledger closed RPC handler.

use app::ApplicationRoot;
use basics::base_uint::Uint256;
use ledger::{Ledger, LedgerHeader};
use rpc::{LedgerClosed, LedgerClosedSource, do_ledger_closed};
use std::sync::Arc;

#[derive(Debug)]
struct FakeClosed {
    ledger: Option<LedgerClosed>,
}

impl LedgerClosedSource for FakeClosed {
    fn closed_ledger(&self) -> Option<LedgerClosed> {
        self.ledger
    }
}

#[test]
fn ledger_closed_returns_closed_index_and_hash() {
    let ledger = LedgerClosed {
        seq: 83_211,
        hash: Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
            .expect("hash should parse"),
    };

    let result = do_ledger_closed(&FakeClosed {
        ledger: Some(ledger),
    });
    let protocol::JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        object.get("ledger_index"),
        Some(&protocol::JsonValue::Unsigned(83_211))
    );
    assert_eq!(
        object.get("ledger_hash"),
        Some(&protocol::JsonValue::String(
            "0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210".to_owned()
        ))
    );
}

#[test]
#[should_panic(expected = "xrpl::doLedgerClosed : non-null closed ledger")]
fn ledger_closed_panics_when_closed_ledger_is_missing_assert() {
    let _ = do_ledger_closed(&FakeClosed { ledger: None });
}

#[test]
fn ledger_closed_reads_app_owned_closed_ledger_through_application_server_info() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    let mut ledger = Ledger::from_ledger_seq_and_close_time(91_007, 55, false);
    ledger.set_ledger_info(LedgerHeader {
        hash: basics::sha_map_hash::SHAMapHash::new(
            Uint256::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
                .expect("hash should parse"),
        ),
        ..ledger.header()
    });
    app.on_closed_ledger(Arc::new(ledger));

    let protocol::JsonValue::Object(object) =
        rpc::do_ledger_closed(&rpc::ApplicationServerInfo::new(&app))
    else {
        panic!("result must be an object");
    };

    assert_eq!(
        object.get("ledger_index"),
        Some(&protocol::JsonValue::Unsigned(91_007))
    );
    assert_eq!(
        object.get("ledger_hash"),
        Some(&protocol::JsonValue::String(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned()
        ))
    );
}

#[test]
fn ledger_closed_response_has_exactly_two_fields() {
    let ledger = LedgerClosed {
        seq: 1,
        hash: Uint256::from_array([0x11; 32]),
    };

    let result = do_ledger_closed(&FakeClosed {
        ledger: Some(ledger),
    });
    let protocol::JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };

    assert_eq!(object.len(), 2);
    assert!(object.contains_key("ledger_index"));
    assert!(object.contains_key("ledger_hash"));
    assert!(!object.contains_key("error"));
}

#[test]
fn ledger_closed_hash_is_64_hex_chars() {
    let ledger = LedgerClosed {
        seq: 999,
        hash: Uint256::from_array([0xFF; 32]),
    };

    let result = do_ledger_closed(&FakeClosed {
        ledger: Some(ledger),
    });
    let protocol::JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };

    let protocol::JsonValue::String(hash) = object.get("ledger_hash").unwrap() else {
        panic!("hash must be a string");
    };
    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}
