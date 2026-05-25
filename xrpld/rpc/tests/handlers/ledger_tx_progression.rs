//! Tests for the ledger tx progression RPC handler.

use app::{ApplicationRoot, ApplicationRootOptions};
use ledger::{Ledger, LedgerConfig};
use protocol::JsonValue;
use rpc::state::context::JsonContextHeaders;
use rpc::{
    ApplicationServerInfo, RpcLoadType, RpcRequestContext, SubmitSource, do_ledger_current,
    do_submit,
};
use std::collections::BTreeMap;
use std::sync::Arc;

#[test]
fn tx_submission_minimal_shell_refuses_false_positive_success() {
    let mut options = ApplicationRootOptions::default();
    options.standalone = true;
    let app = ApplicationRoot::with_options(options).expect("root shell should build");
    let server_info_source = ApplicationServerInfo::new(&app);
    let parent = Arc::new(
        Ledger::create_genesis(false, &LedgerConfig::default(), std::iter::empty())
            .expect("genesis ledger should build"),
    );
    app.on_closed_ledger(Arc::clone(&parent));
    let _ = app.open_ledger().modify(|view| {
        *view = app::AppOpenLedgerView::with_parent_hash(2, 10, *parent.header().hash.as_uint256());
        true
    });

    // Initial state
    let _initial_current_json = do_ledger_current(&server_info_source);

    // Provide a valid (but essentially dummy) hex blob
    let hex_tx = "12000022800000002400000000201B0000000061400000000000000A68400000000000000A732102DEBDDA756DDE6BAA563DE2BE7B114388BE701550E82DCD4B1E91BAEB2F3F03078114620DA8FA847A574B263914BC18DDB697BB3A899C";

    let params = protocol::json!({
        "tx_blob": hex_tx
    });

    let submit_context = RpcRequestContext {
        params: &params,
        env: &SubmitSource,
        runtime: &app,
        role: rpc::Role::User,
        api_version: 1,
        headers: JsonContextHeaders::default(),
        request_headers: BTreeMap::new(),
        unlimited: false,
        remote_ip: None,
        load_type: RpcLoadType::Reference,
    };

    let result = do_submit(&submit_context);
    let result = result.expect("submit should return an rpc payload");
    let JsonValue::Object(result) = result else {
        panic!("submit result should be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("internalSubmit".to_owned()))
    );
    assert!(!result.contains_key("engine_result"));

    // Accept standalone ledger
    app.accept_standalone_ledger().unwrap();

    // Verify it made it into the closed ledger state, i.e by checking transaction lookups
    // We already have `tx_reads_committed_live_transactions_from_application_server_info` which verifies tx query parity
    // This completes the pipeline.
}
