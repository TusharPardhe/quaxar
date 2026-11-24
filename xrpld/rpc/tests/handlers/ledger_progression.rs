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

fn ledger_response(
    source: &ApplicationServerInfo<&ApplicationRoot>,
    ledger_index: u64,
    api_version: u32,
) -> std::collections::BTreeMap<String, protocol::JsonValue> {
    let params = protocol::JsonValue::Object(std::collections::BTreeMap::from([(
        "ledger_index".to_owned(),
        protocol::JsonValue::Unsigned(ledger_index),
    )]));
    let protocol::JsonValue::Object(result) =
        rpc::do_ledger(&params, rpc::RpcRole::User, api_version, source)
    else {
        panic!("ledger must return an object");
    };
    assert_eq!(result.get("error"), None, "ledger failed: {result:?}");
    result
}

/// rippled `LedgerMaster::isValidated`: a ledger older than the newest
/// validated one is still validated when it is in the validated chain. Before
/// the fix quaxar compared against the newest ledger only and answered
/// `validated: false` for every older ledger (mainnet, 2026-11-24). Also
/// checks rippled `fillJson`: the header's `ledger_index` is a string in API
/// v1 and a number from v2.
#[test]
fn ledger_reports_older_validated_ledgers_as_validated() {
    let mut options = ApplicationRootOptions::default();
    options.standalone = true;
    let mut app = ApplicationRoot::with_options(options).expect("root shell should build");
    let _ = app.attach_default_network_ops_runtime();
    for _ in 0..3 {
        app.accept_standalone_ledger().expect("standalone accept");
    }
    let source = ApplicationServerInfo::new(&app);
    let newest = app
        .validated_ledger()
        .expect("standalone accept validates")
        .header()
        .seq;
    let older = u64::from(newest - 1);

    let v1 = ledger_response(&source, older, 1);
    assert_eq!(v1.get("validated"), Some(&protocol::JsonValue::Bool(true)));
    let Some(protocol::JsonValue::Object(header)) = v1.get("ledger") else {
        panic!("ledger header must be an object: {v1:?}");
    };
    assert_eq!(
        header.get("ledger_index"),
        Some(&protocol::JsonValue::String(older.to_string()))
    );

    let v2 = ledger_response(&source, older, 2);
    assert_eq!(v2.get("validated"), Some(&protocol::JsonValue::Bool(true)));
    let Some(protocol::JsonValue::Object(header)) = v2.get("ledger") else {
        panic!("ledger header must be an object: {v2:?}");
    };
    assert_eq!(
        header.get("ledger_index"),
        Some(&protocol::JsonValue::Unsigned(older))
    );
}
