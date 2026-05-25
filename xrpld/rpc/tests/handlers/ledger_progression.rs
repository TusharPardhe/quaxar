//! Tests for the ledger progression RPC handler.

use app::{ApplicationRoot, ApplicationRootOptions};
use rpc::{ApplicationServerInfo, do_ledger_closed, do_ledger_current};

#[test]
fn ledger_state_progresses_after_standalone_accept() {
    let mut options = ApplicationRootOptions::default();
    options.standalone = true;
    let app = ApplicationRoot::with_options(options).expect("root shell should build");
    let server_info_source = ApplicationServerInfo::new(&app);

    // Get initial current ledger index
    let initial_current_json = do_ledger_current(&server_info_source);
    let initial_current_index = match initial_current_json {
        protocol::JsonValue::Object(obj) => match obj.get("ledger_current_index") {
            Some(protocol::JsonValue::Unsigned(val)) => *val,
            _ => panic!("Expected unsigned ledger_current_index"),
        },
        _ => panic!("Expected object from do_ledger_current"),
    };

    // Close ledger
    app.accept_standalone_ledger().unwrap();

    // Get new current ledger index
    let new_current_json = do_ledger_current(&server_info_source);
    let new_current_index = match new_current_json {
        protocol::JsonValue::Object(obj) => match obj.get("ledger_current_index") {
            Some(protocol::JsonValue::Unsigned(val)) => *val,
            _ => panic!("Expected unsigned ledger_current_index"),
        },
        _ => panic!("Expected object from do_ledger_current"),
    };

    assert!(new_current_index > initial_current_index);

    // Verify ledger_closed reflects the progression
    let closed_json = do_ledger_closed(&server_info_source);
    let closed_hash = match closed_json {
        protocol::JsonValue::Object(obj) => match obj.get("ledger_hash") {
            Some(protocol::JsonValue::String(val)) => val.clone(),
            _ => panic!("Expected string ledger_hash"),
        },
        _ => panic!("Expected object from do_ledger_closed"),
    };
    assert!(!closed_hash.is_empty());
}
