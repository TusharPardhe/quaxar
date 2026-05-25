//! Tests for the ledger current RPC handler.

use app::{ApplicationRoot, ServiceRegistry};
use rpc::{LedgerCurrentSource, do_ledger_current};
use std::sync::Arc;

fn sample_open_tx() -> Arc<protocol::STTx> {
    Arc::new(protocol::STTx::new(protocol::TxType::OFFER_CREATE, |_| {}))
}

#[derive(Debug)]
struct FakeCurrent {
    index: u32,
}

impl LedgerCurrentSource for FakeCurrent {
    fn current_ledger_index(&self) -> u32 {
        self.index
    }
}

#[test]
fn ledger_current_returns_current_index() {
    let result = do_ledger_current(&FakeCurrent { index: 42 });
    let protocol::JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        object.get("ledger_current_index"),
        Some(&protocol::JsonValue::Unsigned(42))
    );
}

#[test]
fn ledger_current_reads_app_owned_status_index_through_application_server_info() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_status_rpc_current_ledger_index(Some(88_221));

    let protocol::JsonValue::Object(object) =
        rpc::do_ledger_current(&rpc::ApplicationServerInfo::new(&app))
    else {
        panic!("result must be an object");
    };

    assert_eq!(
        object.get("ledger_current_index"),
        Some(&protocol::JsonValue::Unsigned(88_221))
    );
}

#[test]
fn ledger_current_falls_back_to_live_open_ledger_index_when_status_snapshot_is_missing() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    assert_eq!(app.status_rpc_current_ledger_index(), None);

    let changed = ServiceRegistry::get_open_ledger(&app).modify(|next| {
        next.ledger_current_index = 91_337;
        next.push_transaction(sample_open_tx());
        true
    });
    assert!(changed);

    let protocol::JsonValue::Object(object) =
        rpc::do_ledger_current(&rpc::ApplicationServerInfo::new(&app))
    else {
        panic!("result must be an object");
    };

    assert_eq!(
        object.get("ledger_current_index"),
        Some(&protocol::JsonValue::Unsigned(91_337))
    );
}

#[test]
fn ledger_current_response_has_exactly_one_field() {
    let result = do_ledger_current(&FakeCurrent { index: 100 });
    let protocol::JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };

    assert_eq!(object.len(), 1);
    assert!(object.contains_key("ledger_current_index"));
    assert!(!object.contains_key("error"));
}

#[test]
fn ledger_current_returns_zero_for_zero_index() {
    let result = do_ledger_current(&FakeCurrent { index: 0 });
    let protocol::JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        object.get("ledger_current_index"),
        Some(&protocol::JsonValue::Unsigned(0))
    );
}

#[test]
fn ledger_current_returns_large_index() {
    let result = do_ledger_current(&FakeCurrent { index: u32::MAX });
    let protocol::JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        object.get("ledger_current_index"),
        Some(&protocol::JsonValue::Unsigned(u64::from(u32::MAX)))
    );
}
