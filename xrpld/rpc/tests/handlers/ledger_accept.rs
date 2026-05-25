//! Tests for the ledger accept RPC handler.

use app::ApplicationRoot;
use rpc::{
    JsonContextHeaders, LedgerAcceptSource, Role, RpcLoadType, RpcRequestContext, do_ledger_accept,
};

fn request_context<'a>(
    runtime: &'a ApplicationRoot,
) -> RpcRequestContext<'a, LedgerAcceptSource, ApplicationRoot> {
    RpcRequestContext {
        params: &protocol::JsonValue::Null,
        env: &LedgerAcceptSource,
        runtime,
        role: Role::Admin,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        request_headers: std::collections::BTreeMap::new(),
        unlimited: true,
        remote_ip: None,
        load_type: RpcLoadType::Reference,
    }
}

#[test]
fn ledger_accept_rejects_non_standalone_runtime() {
    // unless the server is launched in standalone mode.
    // If this assertion fires in parity runs, it is an environment setup issue
    // (missing standalone launch), not a submit/tx semantic regression.
    let app = ApplicationRoot::new(0).expect("root shell should build");
    let error = do_ledger_accept(&request_context(&app)).expect_err("non-standalone should fail");

    assert_eq!(error.error_code(), Some(rpc::RpcErrorCode::NotStandalone));
}

#[test]
fn ledger_accept_advances_the_app_owned_current_ledger_index_in_standalone_mode() {
    // closed/validated/current ledger progression.
    let app = ApplicationRoot::with_options(app::ApplicationRootOptions {
        standalone: true,
        ..app::ApplicationRootOptions::default()
    })
    .expect("standalone root should build");

    let result = do_ledger_accept(&request_context(&app)).expect("standalone accept should work");
    let protocol::JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        object.get("ledger_current_index"),
        Some(&protocol::JsonValue::Unsigned(2))
    );
    assert_eq!(app.closed_ledger_seq(), Some(1));
    assert_eq!(app.validated_ledger_seq(), Some(1));
    assert_eq!(app.live_current_ledger_index(), Some(2));
}
